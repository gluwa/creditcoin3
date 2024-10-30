use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::error::Error;

#[derive(Serialize, Deserialize, Debug)]
struct WorkOrderResponse {
    work_order_id: String,
    work_order_status: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct WorkOrderResult {
    work_order_id: String,
    work_order_status: String,
    result: Option<String>,
}

async fn post_work_order(client: &Client) -> Result<WorkOrderResponse, Box<dyn Error>> {
    let url = "http://127.0.0.1:5000/api/prove";

    let form = reqwest::multipart::Form::new()
        .text("private_input_file", "dummy_private_input.json")
        .text("public_input_file", "dummy_public_input.json")
        .text("prover_config_file", "dummy_prover_config.json")
        .text("parameter_file", "dummy_parameter.json");

    let response = client
        .post(url)
        .multipart(form)
        .send()
        .await?
        .json::<WorkOrderResponse>()
        .await?;

    Ok(response)
}

async fn get_work_order_status(
    client: &Client,
    work_order_id: &str,
) -> Result<WorkOrderResponse, Box<dyn Error>> {
    let url = format!("http://127.0.0.1:5000/api/prove/{}", work_order_id);

    let response = client
        .get(&url)
        .send()
        .await?
        .json::<WorkOrderResponse>()
        .await?;

    Ok(response)
}

async fn get_work_order_result(
    client: &Client,
    work_order_id: &str,
) -> Result<WorkOrderResult, Box<dyn Error>> {
    let url = format!("http://127.0.0.1:5000/api/prove/{}/result", work_order_id);

    let response = client
        .get(&url)
        .send()
        .await?
        .json::<WorkOrderResult>()
        .await?;

    Ok(response)
}
