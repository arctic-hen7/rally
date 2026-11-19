use anyhow::{Context, Result, bail, ensure};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use rustix::process::{Pid, Signal, kill_process_group};
use serde::Deserialize;
use serde_json::Value;
use std::{
    io::{ErrorKind, Read},
    os::unix::{net::UnixStream, process::CommandExt},
    process::{Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};
use url::Url;

#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum Source {
    Environment {
        env: String,
    },
    Command {
        command: Vec<String>,
        login_command: Option<Vec<String>>,
        timeout_ms: Option<u64>,
        refresh_interval_ms: Option<u64>,
    },
}

struct Header {
    name: HeaderName,
    source: Source,
    cached: Option<(Instant, HeaderValue)>,
}

pub struct Headers(Vec<Header>);

impl Headers {
    pub fn load(config: &Value, url: &Url) -> Result<Self> {
        let mut entries: Vec<Header> = Vec::new();
        let Some(origins) = config.get("http_headers") else {
            return Ok(Self(entries));
        };
        for (key, settings) in origins
            .as_object()
            .context("Expected an http_headers origin map")?
        {
            let origin = Url::parse(key).context("Invalid http_headers origin")?;
            ensure!(
                matches!(origin.scheme(), "http" | "https")
                    && origin.path() == "/"
                    && origin.query().is_none()
                    && origin.fragment().is_none()
                    && origin.username().is_empty()
                    && origin.password().is_none(),
                "Expected a bare HTTP(S) origin in http_headers"
            );
            if origin.origin() != url.origin() {
                continue;
            }
            for (name, value) in settings
                .as_object()
                .context("Expected a map of HTTP header names")?
            {
                let name =
                    HeaderName::from_bytes(name.as_bytes()).context("Invalid HTTP header name")?;
                ensure!(
                    !matches!(
                        name.as_str(),
                        "authorization"
                            | "host"
                            | "content-length"
                            | "transfer-encoding"
                            | "connection"
                            | "upgrade"
                            | "trailer"
                            | "te"
                            | "proxy-authorization"
                            | "content-type"
                    ),
                    "Rally manages the {name} header"
                );
                ensure!(
                    !entries.iter().any(|header| header.name == name),
                    "Duplicate {name} header"
                );
                let source: Source = serde_json::from_value(value.clone())
                    .map_err(|_| anyhow::anyhow!("Invalid settings for {name}"))?;
                entries.push(Header {
                    name,
                    source,
                    cached: None,
                });
            }
        }
        Ok(Self(entries))
    }

    pub fn resolve(&mut self, login: bool) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        for header in &mut self.0 {
            let value = header
                .value(login)
                .with_context(|| format!("Resolving HTTP header {}", header.name))?;
            headers.insert(header.name.clone(), value);
        }
        Ok(headers)
    }
}

impl Header {
    fn value(&mut self, login: bool) -> Result<HeaderValue> {
        let bytes = match &self.source {
            Source::Environment { env } => std::env::var(env)
                .with_context(|| format!("Missing environment variable {env}"))?
                .into_bytes(),
            Source::Command {
                command,
                login_command,
                timeout_ms,
                refresh_interval_ms,
            } => {
                let interval = Duration::from_millis(refresh_interval_ms.unwrap_or(60_000));
                if let Some((at, value)) = &self.cached
                    && at.elapsed() < interval
                {
                    return Ok(value.clone());
                }
                let timeout = Duration::from_millis(timeout_ms.unwrap_or(15_000));
                let (mut status, mut bytes) = run(command, timeout, false)?;
                if (!status.success() || bytes.iter().all(u8::is_ascii_whitespace))
                    && let Some(command_login) = login_command
                {
                    ensure!(
                        login,
                        "No gateway credential available ({status}). Run rally login in a terminal."
                    );
                    eprintln!("Signing in for HTTP header {}…", self.name);
                    let (result, _) = run(command_login, Duration::from_secs(180), true)?;
                    ensure!(result.success(), "Login command failed ({result})");
                    (status, bytes) = run(command, timeout, false)?;
                }
                ensure!(status.success(), "Lookup command failed ({status})");
                bytes
            }
        };
        ensure!(
            !bytes.iter().all(u8::is_ascii_whitespace),
            "Empty header value"
        );
        let mut value =
            HeaderValue::from_bytes(&bytes).context("Expected a single HTTP header value")?;
        value.set_sensitive(true);
        if matches!(self.source, Source::Command { .. }) {
            self.cached = Some((Instant::now(), value.clone()));
        }
        Ok(value)
    }
}

fn run(argv: &[String], timeout: Duration, login: bool) -> Result<(ExitStatus, Vec<u8>)> {
    let program = argv
        .first()
        .filter(|s| !s.is_empty())
        .context("Empty header command")?;
    let mut cmd = Command::new(program);
    cmd.args(&argv[1..])
        .current_dir(rally_teams::config_dir())
        .process_group(0)
        .stdin(Stdio::null());
    let (mut reader, writer) = UnixStream::pair()?;
    reader.set_nonblocking(true)?;
    if login {
        cmd.stdout(Stdio::null()).stderr(Stdio::inherit());
    } else {
        cmd.stdout(std::os::fd::OwnedFd::from(writer))
            .stderr(Stdio::null());
    }
    let mut child = cmd.spawn().context("Could not start header command")?;
    let start = Instant::now();
    drop(cmd);
    let mut output = Vec::new();
    let result = (|| {
        loop {
            let status = child.try_wait()?;
            if !login {
                // Drain while the helper runs so a full pipe cannot stall it
                let remaining = 16_385 - output.len();
                match (&mut reader)
                    .take(remaining as u64)
                    .read_to_end(&mut output)
                {
                    Err(e) if e.kind() != ErrorKind::WouldBlock => return Err(e.into()),
                    _ => {}
                }
                ensure!(
                    output.len() <= 16_384,
                    "Header command output exceeds 16 KiB"
                );
            }
            if let Some(status) = status {
                if output.last() == Some(&b'\n') {
                    output.pop();
                    if output.last() == Some(&b'\r') {
                        output.pop();
                    }
                }
                return Ok((status, output));
            }
            if start.elapsed() >= timeout {
                bail!("Header command timed out after {} ms", timeout.as_millis());
            }
            std::thread::sleep(
                Duration::from_millis(10).min(timeout.saturating_sub(start.elapsed())),
            );
        }
    })();
    if result.is_err() && child.try_wait()?.is_none() {
        let _ = kill_process_group(Pid::from_child(&child), Signal::KILL);
        child.kill()?;
        child.wait()?;
    }
    result
}
