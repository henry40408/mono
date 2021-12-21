use anyhow::bail;
use failure::ResultExt;
use headless_chrome::Browser;
use reqwest::StatusCode;
use scraper::{Html, Selector};

/// Parameters for scrape
#[derive(Debug)]
pub struct Scraper<'a> {
    /// URL
    pub url: &'a str,
    /// Optional user ID
    pub user_id: Option<i32>,
    /// Overwrite if entry exists?
    pub force: bool,
    /// Scrape with headless Chromium
    pub headless: bool,
}

impl<'a> Scraper<'a> {
    /// Scrape blob or document with URL
    pub fn from_url(url: &'a str) -> Self {
        Self {
            url,
            user_id: None,
            force: false,
            headless: false,
        }
    }

    /// Set user ID
    pub fn with_user_id(mut self, user_id: i32) -> Self {
        self.user_id = Some(user_id);
        self
    }

    /// Set force flag
    pub fn with_force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }

    /// Set headless flag
    pub fn with_headless(mut self, headless: bool) -> Self {
        self.headless = headless;
        self
    }

    /// Scrap document or blob w/ or w/o headless Chromium
    pub async fn scrape(&'a self) -> anyhow::Result<Scraped<'a>> {
        if self.headless {
            self.scrape_with_headless_chromium()
        } else {
            self.scrape_wo_headless_chromium().await
        }
    }

    fn scrape_with_headless_chromium(&self) -> anyhow::Result<Scraped> {
        let browser = Browser::default().compat()?;
        let tab = browser.wait_for_initial_tab().compat()?;

        tab.navigate_to(self.url).compat()?;

        // wait for initial rendering
        tab.wait_until_navigated().compat()?;

        let html_e = tab.wait_for_element("html").compat()?;

        let html_ro = html_e
            .call_js_fn(
                "function () { return document.querySelector('html').outerHTML }",
                false,
            )
            .compat()?;
        let html = match html_ro.value {
            None => bail!("empty HTML document"),
            Some(v) => serde_json::from_value::<String>(v)?,
        };

        let title_ro = html_e
            .call_js_fn("function () { return document.title }", false)
            .compat()?;
        let title = match title_ro.value {
            None => bail!("no title element found"),
            Some(v) => serde_json::from_value::<String>(v)?,
        };

        Ok(Scraped::Document(Document {
            params: self,
            title,
            html,
            http_status: 0, // TODO get actual status code
        }))
    }

    async fn scrape_wo_headless_chromium(&'a self) -> anyhow::Result<Scraped<'a>> {
        let res = reqwest::get(self.url).await?;

        if StatusCode::OK != res.status() && !self.force {
            bail!("failed to fetch response: {}", res.status())
        }

        let http_status = i32::try_from(res.status().as_u16())?;
        let content = res.bytes().await?;

        if infer::is_image(&content) {
            let mime_type = match infer::get(&content) {
                None => bail!("unknown MIME type"),
                Some(t) => t,
            };
            Ok(Scraped::Blob(Blob {
                params: self,
                mime_type,
                content: content.to_vec(),
                http_status,
            }))
        } else {
            let html = String::from_utf8_lossy(&content).to_string();

            let parsed = Html::parse_document(&html);
            let selector = Selector::parse("title").unwrap();

            let title = match parsed.select(&selector).next() {
                None => bail!("no title element found"),
                Some(t) => t.text().collect::<Vec<_>>().join(""),
            };
            Ok(Scraped::Document(Document {
                params: self,
                title,
                html,
                http_status,
            }))
        }
    }
}

/// Scraped blob or document
#[derive(Debug)]
pub enum Scraped<'a> {
    /// e.g. Image
    Blob(Blob<'a>),
    /// e.g. HTML document
    Document(Document<'a>),
}

/// Scraped blob
#[derive(Debug)]
pub struct Blob<'a> {
    /// Parameters
    pub params: &'a Scraper<'a>,
    /// Inferred MIME type
    pub mime_type: infer::Type,
    /// Blob content
    pub content: Vec<u8>,
    /// HTTP status
    pub http_status: i32,
}

/// Scraped document
#[derive(Debug)]
pub struct Document<'a> {
    /// Parameters
    pub params: &'a Scraper<'a>,
    /// Document title
    pub title: String,
    /// Raw HTML document
    pub html: String,
    /// HTTP status
    pub http_status: i32,
}

#[cfg(test)]
mod test {
    use crate::{Scraped, Scraper};

    #[tokio::test]
    async fn test_scrape_with_headless_chromium() -> anyhow::Result<()> {
        let scraper = Scraper::from_url("https://www.example.com").with_headless(true);

        let scraped = scraper.scrape().await?;
        assert!(matches!(scraped, Scraped::Document(_)));

        if let Scraped::Document(doc) = scraped {
            assert_eq!("https://www.example.com", doc.params.url);
            assert!(doc.title.contains("Example Domain"));
            assert!(doc.html.contains("Example Domain"));
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_scrape_wo_headless_chromium() -> anyhow::Result<()> {
        let scraper = Scraper::from_url("https://www.example.com");

        let scraped = scraper.scrape().await?;
        assert!(matches!(scraped, Scraped::Document(_)));

        if let Scraped::Document(doc) = scraped {
            assert_eq!("https://www.example.com", doc.params.url);
            assert!(doc.title.contains("Example Domain"));
            assert!(doc.html.contains("Example Domain"));
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_scrape_image() -> anyhow::Result<()> {
        let scraper = Scraper::from_url("https://picsum.photos/1");

        let scraped = scraper.scrape().await?;
        assert!(matches!(scraped, Scraped::Blob(_)));

        if let Scraped::Blob(blob) = scraped {
            assert_eq!("https://picsum.photos/1", blob.params.url);
            assert!(blob.content.len() > 0);
        }

        Ok(())
    }
}
