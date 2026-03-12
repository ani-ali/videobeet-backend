use axum::{
    extract::{Path, State},
    http::{StatusCode, header},
    response::IntoResponse,
    Json,
};
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};

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

// get single video info by id
pub async fn get_video(
    State(db): State<DbConnection>,
    Path(video_id): Path<String>
) -> impl IntoResponse {
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
                    "error": "database error"
                })),
            ).into_response();
        }
    };

    // increase view count
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
                "playlist_url": format!("/api/stream/{}/playlist.m3u8", video.id),
                "thumbnail_url": video.thumbnail.as_ref().map(|_| format!("/api/videos/{}/thumbnail", video.id)),
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
                "message": "video not found"
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

// get all videos
pub async fn get_all_videos(State(db): State<DbConnection>) -> impl IntoResponse {
    let conn = match db.lock() {
        Ok(conn) => conn,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "database error"
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
                    "error": format!("failed to prepare: {}", e)
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
            // add urls to response
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
                    "thumbnail_url": video.thumbnail.as_ref().map(|_| format!("/api/videos/{}/thumbnail", video.id)),
                    "playlist_url": format!("/api/stream/{}/playlist.m3u8", video.id)
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
                    "error": format!("failed to fetch: {}", e)
                })),
            ).into_response()
        }
    }
}

// serve thumbnail image
pub async fn get_thumbnail(Path(video_id): Path<String>) -> impl IntoResponse {
    let thumbnail_path = format!("videos/output/{}/thumbnail.jpg", video_id);

    match std::fs::read(&thumbnail_path) {
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
                "thumbnail not found",
            ).into_response()
        }
    }
}