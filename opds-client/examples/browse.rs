//! Walks a live catalog from its root, printing what the client parsed.
//!
//! ```sh
//! cargo run -p opds-client --example browse -- <catalog-url> [feed-href]
//! ```

use opds_client::OpdsClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = args
        .next()
        .ok_or("usage: browse <catalog-url> [feed-href]")?;
    let client = OpdsClient::new(root.clone(), reqwest::Client::new());

    let feed = match args.next() {
        Some(href) => client.feed(&href).await?,
        None => client.catalog().await?,
    };

    println!(
        "# {} ({:?} items)",
        feed.metadata.title, feed.metadata.number_of_items
    );
    if let Some(next) = feed.next() {
        println!("next: {}", next.href);
    }
    for entry in feed.all_navigation() {
        println!("[nav] {} -> {}", entry.title(), entry.link.href);
    }
    for publication in feed.all_publications() {
        println!(
            "[pub] {} pages={:?} stream={} downloads={:?} cover={}",
            publication.metadata.title,
            publication.page_count(),
            publication.page_stream().is_some(),
            publication
                .open_access()
                .iter()
                .map(|link| link.essence().unwrap_or_default())
                .collect::<Vec<_>>(),
            publication.cover().is_some(),
        );
    }
    Ok(())
}
