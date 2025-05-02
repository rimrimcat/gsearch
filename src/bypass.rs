use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct BypassRequest {
    cmd: String,
    url: String,
    #[serde(rename = "maxTimeout")]
    max_timeout: u32,
}

#[derive(Serialize, Deserialize)]
pub struct ReturnValue {
    test: String,
}

pub async fn send_to_flaresolverr(port: u16, url: String) -> Result<ReturnValue, reqwest::Error> {
    let client = Client::new();
    let request = BypassRequest {
        cmd: "requests.get".into(),
        url,
        max_timeout: 60000,
    };

    let res = client
        .post(format!("http://localhost:{}/", port))
        .json(&request)
        .send()
        .await?;

    Ok(res.json().await?)
}
