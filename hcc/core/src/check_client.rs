use std::io::Write;
use std::net::TcpStream;
use std::sync::Arc;

use anyhow::Context;
use chrono::{DateTime, SubsecRound, TimeZone, Utc};
use rustls::{ClientConfig, Session};
use x509_parser::parse_x509_certificate;

use crate::check_result::{CheckResult, CheckState};
use std::fmt::Formatter;
use std::time::Instant;

/// Client to check SSL certificate
pub struct CheckClient {
    checked_at: DateTime<Utc>,
    config: Arc<ClientConfig>,
    /// Show elapsed time in milliseconds?
    pub elapsed: bool,
    /// Grace period before certificate actually expires
    pub grace_in_days: i64,
}

impl std::fmt::Debug for CheckClient {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CheckClient {{ checked_at: {:?}, elapsed: {:?}, grace_in_days: {:?} }}",
            self.checked_at, self.elapsed, self.grace_in_days
        )
    }
}

impl Default for CheckClient {
    fn default() -> CheckClient {
        let mut config = rustls::ClientConfig::new();
        config
            .root_store
            .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
        CheckClient {
            checked_at: Utc::now().round_subsecs(0),
            config: Arc::new(config),
            elapsed: false,
            grace_in_days: 7,
        }
    }
}

impl CheckClient {
    /// Check SSL certificate of one domain name
    ///
    /// ```
    /// # use hcc::CheckClient;
    /// let client = CheckClient::default();
    /// client.check_one("sha512.badssl.com");
    /// ```
    pub async fn check_one<'a>(&'a self, domain_name: &'a str) -> anyhow::Result<CheckResult<'a>> {
        let dns_name = webpki::DNSNameRef::try_from_ascii_str(domain_name)?;
        let mut sess = rustls::ClientSession::new(&self.config, dns_name);
        let mut sock = TcpStream::connect(format!("{0}:443", domain_name))?;
        let mut tls = rustls::Stream::new(&mut sess, &mut sock);

        let origin = Instant::now();
        match tls.write(Self::build_http_headers(domain_name).as_bytes()) {
            Ok(_) => (),
            Err(_) => return Ok(CheckResult::expired(domain_name, &self.checked_at)),
        };
        let elapsed = Instant::now() - origin;

        let certificates = tls
            .sess
            .get_peer_certificates()
            .with_context(|| format!("no peer certificates found for {0}", domain_name))?;

        let certificate = certificates
            .first()
            .with_context(|| format!("no certificate found for {0}", domain_name))?;

        let not_after = match parse_x509_certificate(certificate.as_ref()) {
            Ok((_, cert)) => cert.validity().not_after,
            Err(_) => return Ok(CheckResult::default()),
        };
        let not_after = Utc.timestamp(not_after.timestamp(), 0);

        let duration = not_after - self.checked_at;
        let days = duration.num_days();
        let state = if days > self.grace_in_days {
            CheckState::Ok
        } else {
            CheckState::Warning
        };
        Ok(CheckResult {
            state,
            checked_at: self.checked_at.timestamp(),
            days: duration.num_days(),
            domain_name,
            not_after: not_after.timestamp(),
            elapsed: if self.elapsed {
                Some(elapsed.as_millis())
            } else {
                None
            },
        })
    }

    /// Check SSL certificates of multiple domain names
    ///
    /// ```
    /// # use hcc::CheckClient;
    /// let client = CheckClient::default();
    /// client.check_many(&["sha256.badssl.com", "sha256.badssl.com"]);
    /// ```
    pub async fn check_many<'a>(
        &'a self,
        domain_names: &'a [&str],
    ) -> anyhow::Result<Vec<CheckResult<'a>>> {
        let client = Arc::new(self);

        let mut tasks = vec![];
        for domain_name in domain_names {
            let client = client.clone();
            tasks.push(client.check_one(domain_name));
        }

        Ok(futures::future::try_join_all(tasks).await?)
    }

    fn build_http_headers(domain_name: &str) -> String {
        format!(
            concat!(
                "GET / HTTP/1.1\r\n",
                "Host: {0}\r\n",
                "Connection: close\r\n",
                "Accept-Encoding: identity\r\n",
                "\r\n"
            ),
            domain_name
        )
    }
}

#[cfg(test)]
mod test {
    use chrono::{TimeZone, Utc};

    use crate::check_client::CheckClient;
    use crate::check_result::CheckState;

    #[tokio::test]
    async fn test_good_certificate() {
        let now = Utc.timestamp(0, 0);
        let domain_name = "sha512.badssl.com";
        let client = CheckClient::default();
        let result = client.check_one(domain_name).await.unwrap();
        assert!(matches!(result.state, CheckState::Ok));
        assert!(result.checked_at > 0);
        assert!(now < Utc.timestamp(result.not_after, 0));
    }

    #[tokio::test]
    async fn test_bad_certificate() {
        let domain_name = "expired.badssl.com";
        let client = CheckClient::default();
        let result = client.check_one(domain_name).await.unwrap();
        assert!(matches!(result.state, CheckState::Expired));
        assert!(result.checked_at > 0);
        assert_eq!(0, result.not_after);
    }

    #[tokio::test]
    async fn test_check_certificates() -> anyhow::Result<()> {
        let domain_names = vec!["sha512.badssl.com", "expired.badssl.com"];
        let client = CheckClient::default();

        let results = client.check_many(domain_names.as_slice()).await?;
        assert_eq!(2, results.len());

        let result = results.get(0).unwrap();
        assert!(matches!(result.state, CheckState::Ok));

        let result = results.get(1).unwrap();
        assert!(matches!(result.state, CheckState::Expired));

        Ok(())
    }

    #[tokio::test]
    async fn test_check_certificate_with_grace_in_days() {
        let domain_name = "sha512.badssl.com";

        let client = CheckClient::default();
        let result = client.check_one(domain_name).await.unwrap();
        assert!(matches!(result.state, CheckState::Ok));

        let mut client = CheckClient::default();
        client.grace_in_days = result.days + 1;

        let result = client.check_one(domain_name).await.unwrap();
        assert!(matches!(result.state, CheckState::Warning));
    }
}
