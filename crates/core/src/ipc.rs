use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use shared::BrowserCapturePayload;

#[derive(Debug)]
pub struct BrowserBridge {
    pub captures: Receiver<BrowserCapturePayload>,
}

pub fn start_browser_bridge(bind_addr: &str) -> Result<BrowserBridge, String> {
    let listener = TcpListener::bind(bind_addr).map_err(|err| format!("bind failed: {err}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("set_nonblocking failed: {err}"))?;

    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let response = match read_http_request(&mut stream) {
                        Ok(body) => match serde_json::from_str::<BrowserCapturePayload>(&body) {
                            Ok(payload) => {
                                let _ = sender.send(payload);
                                http_response("202 Accepted", "{\"status\":\"queued\"}")
                            }
                            Err(err) => http_response(
                                "400 Bad Request",
                                &format!("{{\"error\":\"invalid json: {err}\"}}"),
                            ),
                        },
                        Err(err) => {
                            http_response("400 Bad Request", &format!("{{\"error\":\"{err}\"}}"))
                        }
                    };

                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(100));
                }
                Err(_) => break,
            }
        }
    });

    Ok(BrowserBridge { captures: receiver })
}

fn read_http_request(stream: &mut TcpStream) -> Result<String, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|err| format!("timeout failed: {err}"))?;

    let mut buffer = vec![0_u8; 16 * 1024];
    let size = stream
        .read(&mut buffer)
        .map_err(|err| format!("read failed: {err}"))?;
    let request = String::from_utf8_lossy(&buffer[..size]);

    let Some((head, body)) = request.split_once("\r\n\r\n") else {
        return Err("missing request body".to_owned());
    };

    if !head.starts_with("POST ") {
        return Err("only POST is supported".to_owned());
    }

    Ok(body.to_owned())
}

fn http_response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[cfg(test)]
mod tests {
    use super::http_response;

    #[test]
    fn response_contains_http_status_line() {
        let response = http_response("202 Accepted", "{\"status\":\"queued\"}");
        assert!(response.starts_with("HTTP/1.1 202 Accepted"));
    }
}
