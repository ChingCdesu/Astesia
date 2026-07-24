use anyhow::Context;

enum Transport {
    Stdio,
    Http { port: u16, auth_token: String },
    VerifySharedCredentials,
}

fn parse_transport() -> anyhow::Result<Option<Transport>> {
    let mut arguments = std::env::args().skip(1);
    let Some(argument) = arguments.next() else {
        return Ok(Some(Transport::Stdio));
    };

    match argument.as_str() {
        "--http-port" => {
            let port = arguments
                .next()
                .context("--http-port requires a port number")?
                .parse::<u16>()
                .context("--http-port must be between 0 and 65535")?;
            anyhow::ensure!(
                arguments.next().is_none(),
                "unexpected arguments after --http-port"
            );
            let auth_token = std::env::var("ASTESIA_MCP_AUTH_TOKEN")
                .context("ASTESIA_MCP_AUTH_TOKEN is required for HTTP transport")?;
            Ok(Some(Transport::Http { port, auth_token }))
        }
        "--version" | "-V" => {
            println!("astesia-mcp {}", env!("CARGO_PKG_VERSION"));
            Ok(None)
        }
        "--verify-shared-credentials" => {
            anyhow::ensure!(
                arguments.next().is_none(),
                "unexpected arguments after --verify-shared-credentials"
            );
            Ok(Some(Transport::VerifySharedCredentials))
        }
        "--help" | "-h" => {
            println!(
                "Astesia MCP server\n\nUSAGE:\n  astesia-mcp\n  astesia-mcp --http-port <PORT>\n\nSTDIO mode is standalone. HTTP mode is managed by the Astesia app, binds only to 127.0.0.1, requires the App-provided authentication and synchronization environment, and exits when its parent stdin closes."
            );
            Ok(None)
        }
        _ => anyhow::bail!("unknown argument: {argument}"),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match parse_transport()? {
        Some(Transport::Stdio) => app_lib::mcp::run_stdio().await,
        Some(Transport::Http { port, auth_token }) => {
            app_lib::mcp::run_http(port, auth_token).await
        }
        Some(Transport::VerifySharedCredentials) => app_lib::mcp::verify_shared_credentials().await,
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_variants_keep_http_secrets_out_of_arguments() {
        let transport = Transport::Http {
            port: 43_677,
            auth_token: "secret".into(),
        };
        match transport {
            Transport::Http { port, auth_token } => {
                assert_eq!(port, 43_677);
                assert_eq!(auth_token, "secret");
            }
            Transport::Stdio => panic!("expected HTTP transport"),
            Transport::VerifySharedCredentials => panic!("expected HTTP transport"),
        }
    }
}
