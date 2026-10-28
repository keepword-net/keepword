//! An empty success isn't the page: compared with it, it would read as the
//! whole page deleted. An empty error page is still a status to record.

use keepword_capture::{CaptureError, HttpCapturer, HttpConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn empty_success_is_a_failed_capture() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let status = if buf[..n].starts_with(b"GET /gone ") {
                    "404 Not Found"
                } else {
                    "200 OK"
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });

    let capturer = HttpCapturer::new(HttpConfig {
        allow_private: true,
        ..HttpConfig::default()
    })
    .unwrap();
    let url = |path: &str| url::Url::parse(&format!("http://127.0.0.1:{port}{path}")).unwrap();
    assert!(matches!(
        capturer.capture(&url("/")).await,
        Err(CaptureError::Empty)
    ));
    let gone = capturer.capture(&url("/gone")).await.unwrap();
    assert_eq!(gone.status, 404);
    assert!(gone.body.is_empty());
}
