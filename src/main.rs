use std::error::Error;

mod browser_utils;

mod search;
use search::new_test_search;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    new_test_search().await?;

    Ok(())
}
