use crate::db::establish_connection_pool;
use agent_twitter_client::scraper::Scraper;
use std::env;

pub fn establish_connection(
) -> r2d2::PooledConnection<diesel::r2d2::ConnectionManager<diesel::PgConnection>> {
    let pool = establish_connection_pool();

    pool.get().unwrap()
}

pub async fn get_test_scraper() -> Scraper {
    dotenv::dotenv().ok();
    let session = env::var("LYNEL_SESSION").expect("LYNEL_SESSION must be set");
    let mut scraper = Scraper::new().await.unwrap();
    scraper.set_from_cookie_string(&session).await.unwrap();
    scraper
}
