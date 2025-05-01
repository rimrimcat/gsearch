use std::error::Error;

mod browser_utils;

mod search;
use search::test_search;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    test_search().await?;

    Ok(())
}
