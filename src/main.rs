use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{StatusCode, header, Method},
    response::IntoResponse,
    routing::{get, post},
};
use tower_http::cors::{CorsLayer, Any};
mod api {
    pub mod get_api;
}
use api::get_api::hello_world;
use serde::{Deserialize, Serialize};
use serde_json;
use std::fs::{create_dir_all, write, read_to_string, read};
use std::process::Command;
use uuid::Uuid;
use rusqlite::{Connection, Result as SqliteResult, params};
use std::sync::{Arc, Mutex};
use chrono::Utc;

type DbConnection = Arc<Mutex<Connection>>;

#[derive(Debug, Serialize, Deserialize)]
struct Video {
    id: String,
    title: String,
    original_filename: String,
    file_extension: String,
    duration: Option<f64>,
    resolution: Option<String>,
    upload_date: String,
    description: Option<String>,
    view_count: i64,
    thumbnail: Option<String>,
}

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
    // Initialize database
    let db = match init_database() {
        Ok(conn) => {
            println!("✅ Database initialized successfully");
            Arc::new(Mutex::new(conn))
        }
        Err(e) => {
            eprintln!("❌ Failed to initialize database: {}", e);
            std::process::exit(1);
        }
    };

    // Configure CORS - Allow all origins and headers for full access
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .expose_headers(Any)
        .max_age(std::time::Duration::from_secs(86400));

    // Create app with route
    let app = Router::new()
        .route("/home/main", get(hello_world))
        .route("/home/main/post", post(post_req))
        .route("/home/main/file", post(accept_form))
        .route("/home/main/video", post(process_video).options(handle_options))
        .route("/stream/:video_id/playlist.m3u8", get(serve_playlist))
        .route("/stream/:video_id/:segment", get(serve_segment))
        .route("/api/video/:video_id", get(get_video_info))
        .route("/api/videos", get(get_all_videos))
        .route("/videos/output/:video_id/thumbnail.jpg", get(serve_thumbnail))
        .with_state(db)
        // Set max body size to 1GB for video uploads
        .layer(DefaultBodyLimit::max(1024 * 1024 * 1024))
        .layer(cors);

    // Run server
    println!("🚀 Server running on http://localhost:3000");
    println!("📁 Max upload size: 1GB");
    println!("🗄️ Database: videos.db");

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

// Handle OPTIONS requests for CORS preflight
async fn handle_options() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
            (header::ACCESS_CONTROL_ALLOW_METHODS, "GET, POST, OPTIONS"),
            (header::ACCESS_CONTROL_ALLOW_HEADERS, "Content-Type, Accept"),
            (header::ACCESS_CONTROL_MAX_AGE, "86400"),
        ],
    )
}

// GET / endpoint - returns image

async fn post_req(Json(payload): Json<CreateUser>) -> (StatusCode, Json<User>) {
    let user = User {
        id: 1337,
        username: payload.username.clone(), // Clone the string
    };
    if user.username == "Anish" {
        // Simpler comparison
        (StatusCode::CREATED, Json(user))
    } else {
        (StatusCode::BAD_REQUEST, Json(user))
    }
}

#[derive(Deserialize)]
struct CreateUser {
    username: String,
}

#[derive(Serialize)]
struct User {
    id: u64,
    username: String,
}

async fn accept_form(mut multipart: Multipart) -> impl IntoResponse {
    // Create downloads directory if it doesn't exist
    if let Err(e) = create_dir_all("downloads") {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create downloads directory: {}", e),
        );
    }

    let mut result = vec![];

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => {
                println!("Error reading multipart field: {}", e);
                if result.is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        format!("Failed to read upload: {}", e),
                    );
                } else {
                    // Partial success - return what was processed
                    result.push(format!("Error: Failed to read remaining fields: {}", e));
                    break;
                }
            }
        };

        let name = field.name().unwrap_or("unknown").to_string();
        let file_name = field.file_name().unwrap_or("unnamed").to_string();
        let content_type = field.content_type().unwrap_or("application/octet-stream").to_string();

        match field.bytes().await {
            Ok(data) => {
                if let Err(e) = write(format!("downloads/{}", file_name), &data) {
                    println!("Error writing file {}: {}", file_name, e);
                    result.push(format!(
                        "Error: Failed to save `{}` (`{}`): {}",
                        name, file_name, e
                    ));
                } else {
                    println!(
                        "Length of `{name}` (`{file_name}`: `{content_type}`) is {} bytes",
                        data.len()
                    );
                    result.push(format!(
                        "Length of `{}` (`{}`: `{}`) is {} bytes",
                        name,
                        file_name,
                        content_type,
                        data.len()
                    ));
                }
            }
            Err(e) => {
                println!("Error reading file bytes for {}: {}", file_name, e);
                result.push(format!(
                    "Error: Failed to read `{}` (`{}`): {}",
                    name, file_name, e
                ));
            }
        }
    }

    if result.is_empty() {
        (StatusCode::BAD_REQUEST, "No files uploaded".to_string())
    } else {
        (StatusCode::OK, result.join("\n"))
    }
}

// Video processing endpoint
async fn process_video(State(db): State<DbConnection>, mut multipart: Multipart) -> impl IntoResponse {
    // Create downloads directory if it doesn't exist
    let id = Uuid::new_v4();
    create_dir_all("videos").unwrap();
    create_dir_all(format!("videos/input/{}", id)).unwrap();
    create_dir_all(format!("videos/output/{}", id)).unwrap();

    println!("📹 Starting video upload for ID: {}", id);

    let mut video_title: Option<String> = None;
    let mut video_description: Option<String> = None;
    let mut video_file_data: Option<(String, Vec<u8>)> = None;

    // First pass: collect all form fields
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => {
                println!("❌ Error reading multipart field: {:?}", e);
                // Clean up directories on failure
                std::fs::remove_dir_all(format!("videos/input/{}", id)).ok();
                std::fs::remove_dir_all(format!("videos/output/{}", id)).ok();
                return (
                    StatusCode::BAD_REQUEST,
                    format!("Failed to read upload: {}. This often happens with large files or connection issues.", e),
                );
            }
        };

        // Get field metadata
        let field_name = field.name().map(|n| n.to_string());
        let content_type = field.content_type().map(|ct| ct.to_string());
        println!("📦 Processing field: {:?} with content-type: {:?}", field_name, content_type);

        // Handle text fields (title and description)
        if let Some(name) = &field_name {
            if name == "title" {
                match field.text().await {
                    Ok(text) => {
                        video_title = Some(text);
                        println!("📝 Title received: {:?}", video_title);
                        continue;
                    }
                    Err(e) => {
                        println!("⚠️ Failed to read title field: {}", e);
                        continue;
                    }
                }
            }

            if name == "description" {
                match field.text().await {
                    Ok(text) => {
                        video_description = Some(text);
                        println!("📝 Description received: {:?}", video_description);
                        continue;
                    }
                    Err(e) => {
                        println!("⚠️ Failed to read description field: {}", e);
                        continue;
                    }
                }
            }
        }

        // Handle file upload - store for processing after all fields are collected
        if let Some(file_name) = field.file_name() {
            let file_name = format!("{}", file_name.to_string());
            println!("📄 Found file: {}", file_name);

            // Check if it's a video file
            if file_name.ends_with(".mp4")
                || file_name.ends_with(".avi")
                || file_name.ends_with(".mov")
                || file_name.ends_with(".mkv")
            {
                // Read and store file data for later processing
                match field.bytes().await {
                    Ok(bytes) => {
                        println!("✅ Successfully read {} bytes", bytes.len());
                        video_file_data = Some((file_name, bytes.to_vec()));
                    },
                    Err(e) => {
                        println!("❌ Error reading file bytes: {:?}", e);
                        // Clean up directories on failure
                        std::fs::remove_dir_all(format!("videos/input/{}", id)).ok();
                        std::fs::remove_dir_all(format!("videos/output/{}", id)).ok();
                        return (
                            StatusCode::BAD_REQUEST,
                            format!("Failed to read file data: {}", e),
                        );
                    }
                }
            } else {
                return (
                    StatusCode::BAD_REQUEST,
                    "Please upload a video file (mp4, avi, mov, mkv)".to_string(),
                );
            }
        }
    }

    // Now validate and process everything after collecting all fields

    // Check if title and description were provided
    let final_title = match video_title {
        Some(t) if !t.trim().is_empty() => t,
        _ => {
            // Clean up directories
            std::fs::remove_dir_all(format!("videos/input/{}", id)).ok();
            std::fs::remove_dir_all(format!("videos/output/{}", id)).ok();
            return (
                StatusCode::BAD_REQUEST,
                "Title is required. Please provide a valid 'title' field in the form data.".to_string(),
            );
        }
    };

    let final_description = match video_description {
        Some(d) if !d.trim().is_empty() => d,
        _ => {
            // Clean up directories
            std::fs::remove_dir_all(format!("videos/input/{}", id)).ok();
            std::fs::remove_dir_all(format!("videos/output/{}", id)).ok();
            return (
                StatusCode::BAD_REQUEST,
                "Description is required. Please provide a valid 'description' field in the form data.".to_string(),
            );
        }
    };

    // Check if video file was provided
    let (file_name, file_data) = match video_file_data {
        Some(data) => data,
        None => {
            // Clean up directories
            std::fs::remove_dir_all(format!("videos/input/{}", id)).ok();
            std::fs::remove_dir_all(format!("videos/output/{}", id)).ok();
            return (
                StatusCode::BAD_REQUEST,
                "No video file uploaded. Please provide a video file.".to_string(),
            );
        }
    };

    let file_extension = file_name.split('.').last().unwrap_or("mp4");

    // Now process the video file
    let temp_path = format!("videos/input/{}/{}", id, file_name);

    println!("💾 Saving file to: {}", temp_path);

    // Write the file
    if let Err(e) = write(&temp_path, &file_data) {
        println!("❌ Error writing file to disk: {}", e);
        std::fs::remove_dir_all(format!("videos/input/{}", id)).ok();
        std::fs::remove_dir_all(format!("videos/output/{}", id)).ok();
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save file: {}", e),
        );
    }

    println!("Processing video: {}", file_name);

    // Use FFmpeg to create HLS chunks for streaming
    // This creates small video segments (chunks) and a playlist file
    let hls_dir = format!("videos/output/{}", id);
    let playlist_path = format!("{}/playlist.m3u8", hls_dir);

    let output = Command::new("ffmpeg")
        .args(&[
            "-i",
            &temp_path, // Input file
            "-c:v",
            "libx264", // Video codec
            "-c:a",
            "aac", // Audio codec
            "-preset",
            "fast",
            "-crf",
            "23",
            "-sc_threshold",
            "0", // Disable scene change detection
            "-g",
            "48", // GOP size (keyframe interval)
            "-keyint_min",
            "48",
            "-hls_time",
            "4", // Each chunk is 4 seconds
            "-hls_playlist_type",
            "vod", // Video on demand
            "-hls_segment_filename",
            &format!("{}/segment_%03d.ts", hls_dir), // Chunk files
            "-y",
            &playlist_path, // Output playlist
        ])
        .output();

    match output {
        Ok(result) => {
            if result.status.success() {
                // Get video metadata using ffprobe
                let probe_output = Command::new("ffprobe")
                    .args(&[
                        "-v", "quiet",
                        "-print_format", "json",
                        "-show_format",
                        "-show_streams",
                        &temp_path,
                    ])
                    .output();

                            let mut duration = None;
                            let mut resolution = None;

                            if let Ok(probe) = probe_output {
                                if let Ok(json_str) = String::from_utf8(probe.stdout) {
                                    if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                        // Extract duration
                                        if let Some(format) = json_val.get("format") {
                                            if let Some(dur) = format.get("duration") {
                                                duration = dur.as_str().and_then(|s| s.parse::<f64>().ok());
                                            }
                                        }
                                        // Extract resolution from video stream
                                        if let Some(streams) = json_val.get("streams").and_then(|s| s.as_array()) {
                                            for stream in streams {
                                                if stream.get("codec_type").and_then(|c| c.as_str()) == Some("video") {
                                                    if let (Some(width), Some(height)) =
                                                        (stream.get("width").and_then(|w| w.as_i64()),
                                                         stream.get("height").and_then(|h| h.as_i64())) {
                                                        resolution = Some(format!("{}x{}", width, height));
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Generate thumbnail at a random point in the video
                            let thumbnail_path = format!("videos/output/{}/thumbnail.jpg", id);
                            let mut thumbnail_generated = false;

                            if let Some(dur) = duration {
                                // Pick a random time between 10% and 90% of video duration
                                let random_second = dur * (0.1 + (rand::random::<f64>() * 0.8));

                                println!("📸 Generating thumbnail at {} seconds", random_second);

                                let thumbnail_output = Command::new("ffmpeg")
                                    .args(&[
                                        "-ss", &format!("{}", random_second),
                                        "-i", &temp_path,
                                        "-vframes", "1",
                                        "-vf", "scale=1280:-1",
                                        "-q:v", "2",
                                        "-y",
                                        &thumbnail_path,
                                    ])
                                    .output();

                                match thumbnail_output {
                                    Ok(result) if result.status.success() => {
                                        println!("✅ Thumbnail generated successfully");
                                        thumbnail_generated = true;
                                    }
                                    Ok(result) => {
                                        let error = String::from_utf8_lossy(&result.stderr);
                                        println!("⚠️ Failed to generate thumbnail: {}", error);
                                    }
                                    Err(e) => {
                                        println!("⚠️ Failed to run thumbnail generation: {}", e);
                                    }
                                }
                            }

                            // Save to database (title and description already validated)
                            let upload_date = Utc::now().to_rfc3339();
                            let thumbnail_filename = if thumbnail_generated {
                                Some(format!("{}/thumbnail.jpg", id))
                            } else {
                                None
                            };

                            if let Ok(conn) = db.lock() {
                                let result = conn.execute(
                                    "INSERT INTO videos (id, title, original_filename, file_extension, duration, resolution, upload_date, description, view_count, thumbnail)
                                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                                    params![
                                        id.to_string(),
                                        final_title,
                                        file_name.clone(),
                                        file_extension,
                                        duration,
                                        resolution,
                                        upload_date,
                                        final_description,
                                        0, // view_count
                                        thumbnail_filename
                                    ],
                                );

                                if let Err(e) = result {
                                    println!("⚠️ Failed to save video metadata to database: {}", e);
                                }
                            }

                            return (
                                StatusCode::OK,
                                format!(
                                    "Video processed successfully! Video ID: {}. Playlist: /stream/{}/playlist.m3u8",
                                    id, id
                                ),
                            );
            } else {
                let error = String::from_utf8_lossy(&result.stderr);
                println!("FFmpeg error: {}", error);
                // Clean up directories on failure
                std::fs::remove_dir_all(format!("videos/input/{}", id)).ok();
                std::fs::remove_dir_all(format!("videos/output/{}", id)).ok();
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to process video: {}", error),
                );
            }
        }
        Err(e) => {
            // Clean up directories on failure
            std::fs::remove_dir_all(format!("videos/input/{}", id)).ok();
            std::fs::remove_dir_all(format!("videos/output/{}", id)).ok();
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to run FFmpeg: {}. Make sure FFmpeg is installed.", e),
            );
        }
    }
}

// Serve the m3u8 playlist file for HLS streaming
async fn serve_playlist(Path(video_id): Path<String>) -> impl IntoResponse {
    let playlist_path = format!("videos/output/{}/playlist.m3u8", video_id);

    match read_to_string(&playlist_path) {
        Ok(content) => {
            // Update relative paths to absolute paths for the segments
            let updated_content = content
                .lines()
                .map(|line| {
                    if line.ends_with(".ts") {
                        // Convert segment_000.ts to /stream/{video_id}/segment_000.ts
                        format!("/stream/{}/{}", video_id, line)
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");

            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/vnd.apple.mpegurl"),
                    (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                ],
                updated_content,
            )
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            [
                (header::CONTENT_TYPE, "text/plain"),
                (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
            ],
            "Playlist not found".to_string(),
        ),
    }
}

// Get video information by ID from database
async fn get_video_info(State(db): State<DbConnection>, Path(video_id): Path<String>) -> impl IntoResponse {
    let conn = match db.lock() {
        Ok(conn) => conn,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [
                    (header::CONTENT_TYPE, "application/json"),
                    (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                ],
                Json(serde_json::json!({
                    "error": "Database connection error"
                })),
            ).into_response();
        }
    };

    // Increment view count and get video data
    let _ = conn.execute(
        "UPDATE videos SET view_count = view_count + 1 WHERE id = ?1",
        params![video_id],
    );

    let video = conn.query_row(
        "SELECT id, title, original_filename, file_extension, duration, resolution, upload_date, description, view_count, thumbnail
         FROM videos WHERE id = ?1",
        params![video_id],
        |row| {
            Ok(Video {
                id: row.get(0)?,
                title: row.get(1)?,
                original_filename: row.get(2)?,
                file_extension: row.get(3)?,
                duration: row.get(4)?,
                resolution: row.get(5)?,
                upload_date: row.get(6)?,
                description: row.get(7)?,
                view_count: row.get(8)?,
                thumbnail: row.get(9)?,
            })
        },
    );

    match video {
        Ok(video) => {
            let playlist_path = format!("videos/output/{}/playlist.m3u8", video_id);
            let playlist_exists = std::path::Path::new(&playlist_path).exists();

            let response = serde_json::json!({
                "id": video.id,
                "title": video.title,
                "original_filename": video.original_filename,
                "file_extension": video.file_extension,
                "duration": video.duration,
                "resolution": video.resolution,
                "upload_date": video.upload_date,
                "description": video.description,
                "view_count": video.view_count,
                "playlist_url": format!("/stream/{}/playlist.m3u8", video.id),
                "status": if playlist_exists { "ready" } else { "processing" }
            });

            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/json"),
                    (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                ],
                Json(response),
            ).into_response()
        }
        Err(_) => {
            let response = serde_json::json!({
                "id": video_id,
                "status": "not_found",
                "message": "Video not found"
            });

            (
                StatusCode::NOT_FOUND,
                [
                    (header::CONTENT_TYPE, "application/json"),
                    (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                ],
                Json(response),
            ).into_response()
        }
    }
}

// Get all videos from database
async fn get_all_videos(State(db): State<DbConnection>) -> impl IntoResponse {
    let conn = match db.lock() {
        Ok(conn) => conn,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Database connection error"
                })),
            ).into_response();
        }
    };

    let mut stmt = match conn.prepare(
        "SELECT id, title, original_filename, file_extension, duration, resolution, upload_date, description, view_count, thumbnail
         FROM videos ORDER BY upload_date DESC"
    ) {
        Ok(stmt) => stmt,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to prepare statement: {}", e)
                })),
            ).into_response();
        }
    };

    let videos: Result<Vec<_>, _> = stmt.query_map([], |row| {
        Ok(Video {
            id: row.get(0)?,
            title: row.get(1)?,
            original_filename: row.get(2)?,
            file_extension: row.get(3)?,
            duration: row.get(4)?,
            resolution: row.get(5)?,
            upload_date: row.get(6)?,
            description: row.get(7)?,
            view_count: row.get(8)?,
            thumbnail: row.get(9)?,
        })
    })
    .and_then(|mapped| mapped.collect());

    match videos {
        Ok(videos) => {
            // Transform videos to include thumbnail URLs
            let videos_with_urls: Vec<serde_json::Value> = videos.iter().map(|video| {
                serde_json::json!({
                    "id": video.id,
                    "title": video.title,
                    "original_filename": video.original_filename,
                    "file_extension": video.file_extension,
                    "duration": video.duration,
                    "resolution": video.resolution,
                    "upload_date": video.upload_date,
                    "description": video.description,
                    "view_count": video.view_count,
                    "thumbnail_url": video.thumbnail.as_ref().map(|_| format!("/videos/output/{}/thumbnail.jpg", video.id)),
                    "playlist_url": format!("/stream/{}/playlist.m3u8", video.id)
                })
            }).collect();

            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/json"),
                    (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                ],
                Json(serde_json::json!({
                    "videos": videos_with_urls,
                    "total": videos.len()
                })),
            ).into_response()
        }
        Err(e) => {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to fetch videos: {}", e)
                })),
            ).into_response()
        }
    }
}

// Serve video thumbnail
async fn serve_thumbnail(Path(video_id): Path<String>) -> impl IntoResponse {
    let thumbnail_path = format!("videos/output/{}/thumbnail.jpg", video_id);

    match read(&thumbnail_path) {
        Ok(content) => {
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "image/jpeg"),
                    (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                    (header::CACHE_CONTROL, "public, max-age=86400"),
                ],
                content,
            ).into_response()
        }
        Err(_) => {
            (
                StatusCode::NOT_FOUND,
                "Thumbnail not found",
            ).into_response()
        }
    }
}

// Serve individual video segments (chunks)
async fn serve_segment(Path((video_id, segment)): Path<(String, String)>) -> impl IntoResponse {
    let segment_path = format!("videos/output/{}/{}", video_id, segment);

    match read(&segment_path) {
        Ok(content) => {
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "video/MP2T"),
                    (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                    (header::CACHE_CONTROL, "public, max-age=3600"),
                ],
                content,
            ).into_response()
        }
        Err(_) => {
            (
                StatusCode::NOT_FOUND,
                "Segment not found",
            ).into_response()
        }
    }
}
