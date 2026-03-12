use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use tower_http::cors::{CorsLayer, Any};
use rusqlite::{Connection, Result as SqliteResult};
use std::sync::{Arc, Mutex};

// import api modules
mod api {
    pub mod health;
    pub mod upload_video;
    pub mod video_info;
    pub mod streaming;
}

// setup database with videos table
fn init_database() -> SqliteResult<Connection> {
    let conn = Connection::open("videos.db")?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS videos (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            original_filename TEXT NOT NULL,
            file_extension TEXT NOT NULL,
            duration REAL,
            resolution TEXT,
            upload_date TEXT NOT NULL,
            description TEXT,
            view_count INTEGER DEFAULT 0,
            thumbnail TEXT
        )",
        [],
    )?;

    Ok(conn)
}

#[tokio::main]
async fn main() {
    // setup database
    let db = match init_database() {
        Ok(conn) => {
            println!("database ready");
            Arc::new(Mutex::new(conn))
        }
        Err(e) => {
            eprintln!("database failed: {}", e);
            std::process::exit(1);
        }
    };

    // setup cors for frontend
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .expose_headers(Any)
        .max_age(std::time::Duration::from_secs(86400));

    // setup all routes - clean and simple
    let app = Router::new()
        // health check
        .route("/api/health", get(api::health::health_check))

        // video endpoints
        .route("/api/videos", get(api::video_info::get_all_videos))
        .route("/api/videos/:id", get(api::video_info::get_video))
        .route("/api/videos/:id/thumbnail", get(api::video_info::get_thumbnail))
        .route("/api/upload", post(api::upload_video::handle_upload))

        // streaming endpoints
        .route("/api/stream/:video_id/playlist.m3u8", get(api::streaming::serve_playlist))
        .route("/api/stream/:video_id/:segment", get(api::streaming::serve_segment))

        .with_state(db)
        // max upload size 1gb
        .layer(DefaultBodyLimit::max(1024 * 1024 * 1024))
        .layer(cors);

    // start server
    println!("server running on http://localhost:3000");
    println!("max upload: 1GB");

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}