use std::env;

use agent_twitter_client::scraper::Scraper;

#[tokio::test]
async fn test_methods() {
    // let session = env::var("REAL_SESSION").unwrap();
    dotenv::dotenv().ok();
    let session = env::var("LYNEL_SESSION").unwrap();

    let mut scraper = Scraper::new().await.unwrap();
    scraper.set_from_cookie_string(&session).await.unwrap();

    // get messages
    let msgs = scraper
        .get_direct_message_conversations("CryptoLynel", None)
        .await
        .unwrap();
    println!("{msgs:?}");
}
