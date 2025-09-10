use super::QueryId;
use anyhow::Result;
use hex::ToHex;
use reqwest::header::{HeaderName, ACCEPT};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use tokio::time::sleep;
use tracing::{debug, info, warn};

const API_KEY: &str = "api-key";
const POST_WORK_ORDER_RETRIES: u32 = 5;
const RETRY_SECONDS: u64 = 30;

// Maps proving input file names to corresponding proving request field names
const FILE_NAME_TO_FIELD_MAP: &[(&str, &str)] = &[
    ("trace.json", "TraceFile"),
    ("memory.json", "MemoryFile"),
    ("private_input.json", "PrivateInputFile"),
    ("public_input.json", "PublicInputFile"),
    ("program_input.json", "ProgramInputFile"),
    ("output.txt", "OutputFile"),
];

fn get_request_field(file_name: &str) -> Result<String> {
    Ok(FILE_NAME_TO_FIELD_MAP
        .iter()
        .find(|(file, _)| *file == file_name)
        .ok_or(Error::BadProofInputFile(file_name.to_string()))?
        .1
        .to_string())
}

#[derive(Serialize, Deserialize, Debug)]
struct WorkOrderResponse {
    request_id: String,
    query_id: String,
    message: String,
    status: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct PipelineStatusResponse {
    run_id: String,
    status: String,
    message: String,
    start_time: String,
    end_time: String,
    duration_in_ms: String,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("The following file has no request field mapping: {0}")]
    BadProofInputFile(String),
    #[error("Sending request failed. Check that `prover-be-socket-addr` argument is provided and correct. Error: {0}")]
    ReqwestSendError(String),
    #[error("Attempted to send proving request {0} times. Out of attempts.")]
    ReqwestSendOutOfRetries(u32),
    #[error("BadProofOrderRequest. Error: {0}")]
    BadProofOrderRequest(String),
    #[error("Couldn't parse work order response. Error: {0}")]
    BadProofOrderResponse(String),
    #[error("Timeout reached: Result not available within 60 minutes")]
    ProvingPipelineTimeout,
    #[error("The Prover BE deleted our proving job completely rather than setting it to a failed state. This indicates the issue is with the prover BE and not with our query.")]
    ProofGenerationFailed,
    #[error("Bad proof result request. StatusCode: {0}")]
    BadProofResultRequest(String),
    #[error("Bad proof result response. Error: {0}")]
    BadProofResultResponse(String),
    #[error("Form preparation failed: {0}")]
    FormPreparationFailed(String),
}

type Proof = Vec<u8>;

/// Handle proof order
pub async fn handle_proof_order(
    query_id: QueryId,
    files: Vec<PathBuf>,
    prover_be_socket_addr: &str,
    be_api_key: &str,
) -> Result<Proof, Error> {
    let client = Client::new();

    let response = build_and_post_order_with_retries(
        &client,
        query_id,
        files,
        prover_be_socket_addr,
        be_api_key,
    )
    .await?;

    // Poll for the result
    let proof_bytes = poll_for_result(&client, &response.query_id, prover_be_socket_addr).await?;
    debug!("Work order proof len: {}", proof_bytes.len());
    Ok(proof_bytes)
}

async fn build_and_post_order_with_retries(
    client: &Client,
    query_id: QueryId,
    files: Vec<PathBuf>,
    prover_be_socket_addr: &str,
    be_api_key: &str,
) -> std::result::Result<WorkOrderResponse, Error> {
    // Internet connection interruptions should be tolerated, rather than treated as proving failures
    for i in 0..POST_WORK_ORDER_RETRIES {
        let form = prepare_proof_order_form(query_id, &files).await?;
        let response_or_err =
            post_work_order(client, form, prover_be_socket_addr, be_api_key).await;
        if let Err(error) = response_or_err {
            match error {
                Error::ReqwestSendError(message) => {
                    warn!("⚠️ Sending proving request to BE failed. Make sure prover has stable internet. Error: {:?}", message);
                }
                _ => return Err(error),
            }
        } else {
            return response_or_err;
        }
        // Don't delay winding up if this was the last retry
        if i < POST_WORK_ORDER_RETRIES - 1 {
            sleep(Duration::from_secs(RETRY_SECONDS)).await;
        }
    }
    Err(Error::ReqwestSendOutOfRetries(POST_WORK_ORDER_RETRIES))
}

async fn post_work_order(
    client: &Client,
    form: reqwest::multipart::Form,
    prover_be_socket_addr: &str,
    be_api_key: &str,
) -> std::result::Result<WorkOrderResponse, Error> {
    let url = format!("{prover_be_socket_addr}/AzureAppService/QueueLightProverQueryRequest/prove");
    let response = client
        .post(&url)
        .header(ACCEPT, "*/*")
        .header(HeaderName::from_static(API_KEY), be_api_key)
        .multipart(form)
        .send()
        .await
        .map_err(|e| Error::ReqwestSendError(e.to_string()))?;

    match response.status() {
        reqwest::StatusCode::OK => {
            // Printing the response before parsing is useful for debugging. But normally doing so consumes the
            // response. So we buffer it as bytes first.
            let bytes = response
                .bytes()
                .await
                .map_err(|e| Error::BadProofOrderResponse(e.to_string()))?;

            debug!(
                "📝 Received post_work_order response: {:?}",
                String::from_utf8_lossy(&bytes)
            );

            Ok(serde_json::from_slice::<WorkOrderResponse>(&bytes)
                .map_err(|e| Error::BadProofOrderResponse(e.to_string()))?)
        }
        other_status => Err(Error::BadProofOrderRequest(other_status.to_string())),
    }
}

async fn poll_for_result(
    client: &Client,
    query_id: &str,
    prover_be_socket_addr: &str,
) -> std::result::Result<Vec<u8>, Error> {
    let url = format!(
        "{prover_be_socket_addr}/AzureAppService/GetProverOutput/prover-output/{query_id}",
    );

    let timeout = Duration::from_secs(60 * 60); // 60 minutes
    let interval = Duration::from_secs(RETRY_SECONDS); // Poll every 30 seconds
    let start = tokio::time::Instant::now();

    while start.elapsed() < timeout {
        // If response is Ok but no proof is supplied, then pipeline is still in progress.
        let result = get_work_order_result(client, &url).await;
        match result {
            Ok(maybe_proof) => {
                if let Some(proof) = maybe_proof {
                    return Ok(proof);
                }
            }
            Err(error) => match error {
                Error::ReqwestSendError(message) => {
                    warn!("⚠️ Polling BE for proof result failed. Make sure prover has stable internet. Error: {:?}", message);
                }
                _ => return Err(error),
            },
        }

        info!(
            "🚧 Result not yet available... QueryId: 0x{}, Elapsed: {:?}, Timeout: {:?}",
            query_id,
            start.elapsed().as_secs(),
            timeout.as_secs()
        );
        sleep(interval).await;
    }

    Err(Error::ProvingPipelineTimeout)
}

async fn get_work_order_result(
    client: &Client,
    url: &str,
) -> std::result::Result<Option<Vec<u8>>, Error> {
    let response = client
        .get(url)
        .header(ACCEPT, "*/*")
        .send()
        .await
        .map_err(|e| Error::ReqwestSendError(e.to_string()))?;

    match response.status() {
        reqwest::StatusCode::OK => Ok(Some(
            response
                .bytes()
                .await
                .map_err(|e| Error::BadProofResultResponse(e.to_string()))?
                .into(),
        )),
        // Result not available yet. Pipeline still in progress.
        reqwest::StatusCode::BAD_REQUEST => Ok(None),
        reqwest::StatusCode::NOT_FOUND => Err(Error::ProofGenerationFailed),
        other_status => Err(Error::BadProofResultRequest(other_status.to_string())),
    }
}

async fn prepare_proof_order_form(
    query_id: QueryId,
    files: &Vec<PathBuf>,
) -> Result<reqwest::multipart::Form, Error> {
    // Prepare each file for the multipart form
    let mut form = reqwest::multipart::Form::new();
    for file in files {
        let file_content = tokio::fs::read(&file)
            .await
            .map_err(|e| Error::FormPreparationFailed(e.to_string()))?;
        let filename = file
            .file_name()
            .ok_or("Invalid file name")
            .map_err(|e| Error::FormPreparationFailed(e.to_string()))?;

        let filename_string = filename
            .to_str()
            .ok_or("Invalid file name")
            .map_err(|e| Error::FormPreparationFailed(e.to_string()))?
            .to_string();

        let request_field = match get_request_field(&filename_string) {
            Ok(field_name) => field_name,
            Err(e) => {
                warn!("⚠️ Unexpected file in proof inputs dir. Error: {:?}", e);
                continue;
            }
        };

        form = form.part(
            request_field,
            reqwest::multipart::Part::bytes(file_content).file_name(filename_string),
        );
    }

    let query_id_string: String = query_id.encode_hex();
    info!("📝 Posting work order with query_id: {}", query_id_string);

    // Add query id to the form
    form = form.text("queryId", query_id_string);
    Ok(form)
}
