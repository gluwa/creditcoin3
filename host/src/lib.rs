#![cfg_attr(not(feature = "std"), no_std)]

use sp_runtime_interface::runtime_interface;

#[runtime_interface]
pub trait HostApi {
    fn verify_proof(proof: Vec<u8>) -> bool {
        let result = call_verifier(proof);
        println!("result of verifying proof: {:?}", result);

        true
    }
}

extern crate nix;

use nix::sys::wait::waitpid;
use nix::unistd::{execv, fork, pipe, read, write, ForkResult, Pid};
use std::ffi::CString;
use std::os::fd::AsRawFd;

const SCRIPT_PATH: &'static str = "./verify.sh";

fn call_verifier(proof: Vec<u8>) -> Result<Vec<u8>, ()> {
    // Create a pipe to capture the child's output
    let (reader, writer) = pipe().expect("Failed to create pipe");

    match unsafe { fork() } {
        Ok(ForkResult::Parent { child }) => {
            // In the parent process, close the write end of the pipe
            nix::unistd::close(writer.as_raw_fd()).expect("Failed to close write end of pipe");

            // Read the output from the child process
            let mut output = String::new();
            let mut buf = [0; 1024];
            loop {
                match read(reader.as_raw_fd(), &mut buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => output.push_str(&String::from_utf8_lossy(&buf[..n])),
                    Err(_) => break,
                }
            }

            // Wait for the child process to finish
            let _ = waitpid(child, None).expect("Failed to wait on child process");

            // Print the captured output
            println!("Output from child process: {}", output);
        }
        Ok(ForkResult::Child) => {
            // In the child process, close the read end of the pipe
            nix::unistd::close(reader.as_raw_fd()).expect("Failed to close read end of pipe");

            // Redirect stdout to the write end of the pipe
            nix::unistd::dup2(writer.as_raw_fd(), nix::libc::STDOUT_FILENO).expect("dup2 failed");

            // Execute the shell script with the byte array
            let script = CString::new(SCRIPT_PATH.as_bytes()).expect("CString::new failed");
            let arg1 = CString::new(proof).expect("CString::new failed");
            let args = [script.clone(), arg1];

            execv(&script, &args).expect("execv failed");
        }
        Err(_) => {
            eprintln!("Fork failed");
        }
    }

    Ok(vec![])
}
