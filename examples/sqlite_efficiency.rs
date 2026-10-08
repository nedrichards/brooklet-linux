//! Reproducible prepared-statement comparison on identical warmed data.
use rusqlite::{Connection, Result, params};
use std::time::Instant;

fn lookups(db: &Connection, cached: bool) -> Result<f64> {
    let start = Instant::now();
    let sql = "SELECT title, published FROM articles WHERE account_id=?1 AND id=?2";
    for i in 0..10_000 {
        let id = i % 2000;
        let row: (String, i64) = if cached {
            db.prepare_cached(sql)?
                .query_row(params![1, id], |row| Ok((row.get(0)?, row.get(1)?)))?
        } else {
            db.prepare(sql)?
                .query_row(params![1, id], |row| Ok((row.get(0)?, row.get(1)?)))?
        };
        std::hint::black_box(row);
    }
    Ok(start.elapsed().as_secs_f64() * 1000.0)
}
fn main() -> Result<()> {
    let mut db = Connection::open_in_memory()?;
    db.set_prepared_statement_cache_capacity(32);
    db.execute_batch("CREATE TABLE articles(account_id INTEGER, id INTEGER, title TEXT, published INTEGER, PRIMARY KEY(account_id,id));")?;
    let tx = db.transaction()?;
    for i in 0..2000 {
        tx.execute(
            "INSERT INTO articles VALUES(1,?1,?2,?1)",
            params![i, format!("Article {i}")],
        )?;
    }
    tx.commit()?;
    lookups(&db, false)?;
    lookups(&db, true)?;
    let mut uncached = Vec::new();
    let mut cached = Vec::new();
    for pass in 0..12 {
        for mode in if pass % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let time = lookups(&db, mode)?;
            if mode {
                cached.push(time)
            } else {
                uncached.push(time)
            }
        }
    }
    uncached.sort_by(f64::total_cmp);
    cached.sort_by(f64::total_cmp);
    println!("10,000 identical point reads; 12 alternating warmed passes");
    println!(
        "prepare median {:.3} ms; prepare_cached median {:.3} ms",
        (uncached[5] + uncached[6]) / 2.0,
        (cached[5] + cached[6]) / 2.0
    );
    summary_pages();
    Ok(())
}

fn summary_pages() {
    use brooklet::{
        model::{Account, Entry, SUMMARY_PAGE_SIZE},
        services::traits::Repository,
        storage::sqlite::SqliteRepository,
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let repo = SqliteRepository::open_in_memory().unwrap();
        repo.save_account(&Account {
            id: 1,
            server_url: "https://example.com".into(),
            username: "benchmark".into(),
            server_version: "2.3".into(),
        })
        .await
        .unwrap();
        let entries = (1..=20_000)
            .map(|id| Entry {
                id,
                account_id: 1,
                feed_id: 1,
                feed_title: "Feed".into(),
                category_title: "Category".into(),
                title: format!("Article {id}"),
                url: format!("https://example.com/{id}"),
                author: None,
                published_at_ms: id,
                html: "<p>Body</p>".repeat(32),
                content_revision: 0,
                read: false,
                starred: false,
                reading_minutes: 1,
                delivery_state: None,
                delivery_error: None,
            })
            .collect::<Vec<_>>();
        repo.merge_changed_page(1, &entries, &[]).await.unwrap();
        repo.entries_for_view(1, "all").await.unwrap();
        repo.entries_page(1, "all", None, SUMMARY_PAGE_SIZE)
            .await
            .unwrap();
        let mut full = Vec::new();
        let mut paged = Vec::new();
        for pass in 0..12 {
            for page in if pass % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let start = Instant::now();
                let count = if page {
                    std::hint::black_box(
                        repo.entries_page(1, "all", None, SUMMARY_PAGE_SIZE)
                            .await
                            .unwrap(),
                    )
                    .entries
                    .len()
                } else {
                    std::hint::black_box(repo.entries_for_view(1, "all").await.unwrap()).len()
                };
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                if page {
                    assert_eq!(count, 128);
                    paged.push(elapsed);
                } else {
                    assert_eq!(count, 20_000);
                    full.push(elapsed);
                }
            }
        }
        full.sort_by(f64::total_cmp);
        paged.sort_by(f64::total_cmp);
        println!("20,000 body-free summaries; 12 alternating warmed passes");
        println!(
            "full median {:.3} ms; first 128 median {:.3} ms",
            (full[5] + full[6]) / 2.0,
            (paged[5] + paged[6]) / 2.0
        );
    });
}
