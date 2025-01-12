use actix_web::{get, Responder};
use actix_web_lab::sse;
use futures::stream::Stream;
use std::time::Duration;

#[get("/mcp/stream")]
pub async fn mcp_stream() -> impl Responder {
    let stream = sse_stream();
    sse::Sse::from_stream(stream).with_retry_duration(Duration::from_secs(10))
}

fn sse_stream() -> impl Stream<Item = Result<sse::Event, actix_web::Error>> {
    futures::stream::unfold(0, move |count| async move {
        let data = format!("Message {}", count);
        Some((Ok(sse::Event::Data(sse::Data::new(data))), count + 1))
    })
}
