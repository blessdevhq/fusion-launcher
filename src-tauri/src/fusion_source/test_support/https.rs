//! Real loopback HTTPS with an exact per-fixture endpoint and a test-only root.
//! No certificate bypass, environment override, or production network exception.
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

static PORTS: LazyLock<Mutex<BTreeSet<u16>>> = LazyLock::new(|| Mutex::new(BTreeSet::new()));

pub(crate) fn configure_client(
    builder: reqwest::ClientBuilder,
    url: &url::Url,
) -> reqwest::ClientBuilder {
    let port = url.port_or_known_default().unwrap_or(443);
    if url.scheme() == "https"
        && url.host_str() == Some("acquisition.test")
        && PORTS.lock().unwrap().contains(&port)
    {
        builder
            .no_proxy()
            .add_root_certificate(
                reqwest::Certificate::from_pem(include_bytes!("https/test-cert.pem")).unwrap(),
            )
            .resolve("acquisition.test", ([127, 0, 0, 1], port).into())
    } else {
        builder
    }
}

pub(crate) struct HttpsFixture {
    child: Child,
    port: u16,
}
impl HttpsFixture {
    pub(crate) fn start(root: &Path) -> Self {
        let support =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fusion_source/test_support/https");
        let python = std::env::var_os("FUSION_TEST_PYTHON").unwrap_or_else(|| "python".into());
        let child = Command::new(python)
            .arg(support.join("server.py"))
            .arg(root)
            .arg(support.join("test-cert.pem"))
            .arg(support.join("test-key.pem"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut fixture = Self { child, port: 0 };
        let stdout = fixture.child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
            let _ = tx.send(result);
        });
        let line = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("TLS server readiness")
            .unwrap();
        fixture.port = line.trim().parse().expect("TLS server port");
        PORTS.lock().unwrap().insert(fixture.port);
        fixture
    }
    pub(crate) fn url(&self, mode: &str) -> String {
        format!("https://acquisition.test:{}/{mode}", self.port)
    }
}
impl Drop for HttpsFixture {
    fn drop(&mut self) {
        PORTS.lock().unwrap().remove(&self.port);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
