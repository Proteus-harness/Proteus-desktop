//! Decode a command envelope once, using the same DTO as the server.
use proteus_contracts::app_protocol::StdioOutput;
use serde::de::DeserializeOwned;

pub fn command_output<T: DeserializeOwned>(response: StdioOutput) -> Result<T, String> {
    let output = command_value(response)?.ok_or("command succeeded without output")?;
    serde_json::from_value(output).map_err(|error| format!("invalid command output: {error}"))
}

pub fn command_value(response: StdioOutput) -> Result<Option<serde_json::Value>, String> {
    match response {
        StdioOutput::Response {
            ok: true, output, ..
        } => Ok(output),
        StdioOutput::Response { error, .. } => {
            Err(error.unwrap_or_else(|| "command failed without an error".into()))
        }
        StdioOutput::Event { .. } => Err("unexpected event instead of command response".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acknowledgement_accepts_an_empty_successful_response() {
        let response = StdioOutput::Response {
            id: None,
            ok: true,
            output: None,
            error: None,
        };
        assert_eq!(command_value(response.clone()), Ok(None));
        assert!(command_output::<serde_json::Value>(response).is_err());
    }
}
