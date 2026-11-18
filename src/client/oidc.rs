use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, BufReader},
    net::TcpListener,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub fn login(client: &Client, manual: bool) -> Result<String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let verifier = rally_teams::secret();
    let state = rally_teams::secret();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = url::Url::parse(&format!("{}/auth/oidc/start", client.url))?;
    url.query_pairs_mut()
        .append_pair("port", &port.to_string())
        .append_pair("challenge", &challenge)
        .append_pair("client_state", &state);
    eprintln!("Open this URL to sign in:\n{url}");
    if !manual {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let _ = Command::new(opener)
            .arg(url.as_str())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    listener.set_nonblocking(true)?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(300) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(3)))?;
                let mut line = String::new();
                BufReader::new(&stream).take(8192).read_line(&mut line)?;
                let Some(target) = line
                    .strip_prefix("GET ")
                    .and_then(|s| s.split_whitespace().next())
                else {
                    continue;
                };
                let url = url::Url::parse(&format!("http://127.0.0.1:{port}{target}"))?;
                let params = url
                    .query_pairs()
                    .collect::<std::collections::HashMap<_, _>>();
                if url.path() != "/rally" || params.get("state").map(|v| v.as_ref()) != Some(&state)
                {
                    continue;
                }
                let code = params.get("code").context("Missing login code")?;
                let r = client.send("auth/exchange", &json!({"code":code,"verifier":verifier}))?;
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nSigned in to Rally. You can close this window.\n")?;
                return Ok(field(&r, "token")?.to_owned());
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(e) => return Err(e.into()),
        }
    }
    bail!("Browser sign-in timed out")
}
