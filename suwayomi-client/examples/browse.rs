//! Prints what a live Suwayomi server serves:
//! `cargo run -p suwayomi-client --example browse -- <server-url> [manga-id [chapter-id]]`
//! lists the categories and their mangas, a manga's chapters, or a chapter's
//! pages.

use suwayomi_client::client::{Auth, SuwayomiClient};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let url = args
        .next()
        .ok_or("usage: browse <server-url> [manga-id [chapter-id]]")?;
    let manga_id = args.next().map(|id| id.parse::<i32>()).transpose()?;
    let chapter_id = args.next().map(|id| id.parse::<i32>()).transpose()?;

    let client = SuwayomiClient::new(url, Auth::None)?;
    let about = client.about().await?;
    println!("{} {}", about.name, about.version);

    match (manga_id, chapter_id) {
        (None, _) => {
            for category in client.list_categories().await? {
                println!(
                    "category {} {:?} ({} mangas)",
                    category.id, category.name, category.mangas.total_count
                );
                for manga in client.list_category_mangas(category.id).await? {
                    println!("  manga {} {:?}", manga.id, manga.title);
                }
            }
        }
        (Some(manga_id), None) => {
            for chapter in client.list_chapters(manga_id).await? {
                println!(
                    "chapter {} #{} {:?} ({} pages)",
                    chapter.id, chapter.source_order, chapter.name, chapter.page_count
                );
            }
        }
        (Some(_), Some(chapter_id)) => {
            let pages = client.fetch_chapter_pages(chapter_id).await?;
            println!("{} pages: {:?}", pages.pages.len(), pages.pages);

            let chapter = &pages.chapter;
            let first = client
                .chapter_page(chapter.manga_id, chapter.source_order, 0)
                .await?;
            println!("first page is {} bytes", first.len());
        }
    }

    Ok(())
}
