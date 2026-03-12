use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    response::IntoResponse,
};
use std::fs::{create_dir_all, write};
use std::process::Command;
use uuid::Uuid;
use rusqlite::params;
use std::sync::{Arc, Mutex};
use chrono::Utc;
use rusqlite::Connection;
use tokio::task;

type DbConnection = Arc<Mutex<Connection>>;

// this handles video uploads and processing
pub async fn handle_upload(
    State(db): State<DbConnection>,
    mut multipart: Multipart
) -> impl IntoResponse {
    // generate unique id for video
    let id = Uuid::new_v4();
    create_dir_all("videos").unwrap();
    create_dir_all(format!("videos/input/{}", id)).unwrap();
    create_dir_all(format!("videos/output/{}", id)).unwrap();

    println!("starting video upload for id: {}", id);

    let mut video_title: Option<String> = None;
    let mut video_description: Option<String> = None;
    let mut video_file_data: Option<(String, Vec<u8>)> = None;

    // read all the form fields
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => {
                println!("error reading field: {:?}", e);
                // cleanup on error
                std::fs::remove_dir_all(format!("videos/input/{}", id)).ok();
                std::fs::remove_dir_all(format!("videos/output/{}", id)).ok();
                return (
                    StatusCode::BAD_REQUEST,
                    format!("failed to read upload: {}", e),
                );
            }
        };

        let field_name = field.name().map(|n| n.to_string());
        let content_type = field.content_type().map(|ct| ct.to_string());
        println!("processing field: {:?} type: {:?}", field_name, content_type);

        // check if its title field
        if let Some(name) = &field_name {
            if name == "title" {
                match field.text().await {
                    Ok(text) => {
                        video_title = Some(text);
                        println!("got title: {:?}", video_title);
                        continue;
                    }
                    Err(e) => {
                        println!("failed to read title: {}", e);
                        continue;
                    }
                }
            }

            // check if its description
            if name == "description" {
                match field.text().await {
                    Ok(text) => {
                        video_description = Some(text);
                        println!("got description: {:?}", video_description);
                        continue;
                    }
                    Err(e) => {
                        println!("failed to read description: {}", e);
                        continue;
                    }
                }
            }
        }

        // handle the actual video file
        if let Some(file_name) = field.file_name() {
            let file_name = file_name.to_string();
            println!("found file: {}", file_name);

            // make sure its a video
            if file_name.ends_with(".mp4")
                || file_name.ends_with(".avi")
                || file_name.ends_with(".mov")
                || file_name.ends_with(".mkv")
            {
                match field.bytes().await {
                    Ok(bytes) => {
                        println!("read {} bytes", bytes.len());
                        video_file_data = Some((file_name, bytes.to_vec()));
                    },
                    Err(e) => {
                        println!("error reading file: {:?}", e);
                        std::fs::remove_dir_all(format!("videos/input/{}", id)).ok();
                        std::fs::remove_dir_all(format!("videos/output/{}", id)).ok();
                        return (
                            StatusCode::BAD_REQUEST,
                            format!("failed to read file: {}", e),
                        );
                    }
                }
            } else {
                return (
                    StatusCode::BAD_REQUEST,
                    "upload a video file (mp4, avi, mov, mkv)".to_string(),
                );
            }
        }
    }

    // make sure we got title
    let final_title = match video_title {
        Some(t) if !t.trim().is_empty() => t,
        _ => {
            std::fs::remove_dir_all(format!("videos/input/{}", id)).ok();
            std::fs::remove_dir_all(format!("videos/output/{}", id)).ok();
            return (
                StatusCode::BAD_REQUEST,
                "title is required".to_string(),
            );
        }
    };

    // make sure we got description
    let final_description = match video_description {
        Some(d) if !d.trim().is_empty() => d,
        _ => {
            std::fs::remove_dir_all(format!("videos/input/{}", id)).ok();
            std::fs::remove_dir_all(format!("videos/output/{}", id)).ok();
            return (
                StatusCode::BAD_REQUEST,
                "description is required".to_string(),
            );
        }
    };

    // make sure we got the video file
    let (file_name, file_data) = match video_file_data {
        Some(data) => data,
        None => {
            std::fs::remove_dir_all(format!("videos/input/{}", id)).ok();
            std::fs::remove_dir_all(format!("videos/output/{}", id)).ok();
            return (
                StatusCode::BAD_REQUEST,
                "no video file uploaded".to_string(),
            );
        }
    };

    let file_extension = file_name.split('.').last().unwrap_or("mp4");
    let temp_path = format!("videos/input/{}/{}", id, file_name);

    println!("saving file to: {}", temp_path);

    // save the file
    if let Err(e) = write(&temp_path, &file_data) {
        println!("error saving file: {}", e);
        std::fs::remove_dir_all(format!("videos/input/{}", id)).ok();
        std::fs::remove_dir_all(format!("videos/output/{}", id)).ok();
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to save: {}", e),
        );
    }

    // Save video to database with "processing" status
    let upload_date = Utc::now().to_rfc3339();
    if let Ok(conn) = db.lock() {
        let result = conn.execute(
            "INSERT INTO videos (id, title, original_filename, file_extension, duration, resolution, upload_date, description, view_count, thumbnail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id.to_string(),
                final_title.clone(),
                file_name.clone(),
                file_extension,
                None::<f64>, // duration - will update after processing
                None::<String>, // resolution - will update after processing
                upload_date,
                final_description.clone(),
                0,
                None::<String> // thumbnail - will update after processing
            ],
        );

        if let Err(e) = result {
            println!("failed to save initial record: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to save: {}", e),
            );
        }
    }

    // Clone values for async processing
    let id_clone = id;
    let temp_path_clone = temp_path.clone();
    let file_name_clone = file_name.clone();
    let db_clone = db.clone();

    // Spawn background task for video processing
    task::spawn_blocking(move || {
        println!("background processing started for: {}", file_name_clone);

        // use ffmpeg to create hls chunks
        let hls_dir = format!("videos/output/{}", id_clone);
        let playlist_path = format!("{}/playlist.m3u8", hls_dir);

        let output = Command::new("ffmpeg")
        .args(&[
            "-i", &temp_path_clone,
            "-c:v", "libx264",
            "-c:a", "aac",
            "-preset", "fast",
            "-crf", "23",
            "-sc_threshold", "0",
            "-g", "48",
            "-keyint_min", "48",
            "-hls_time", "4",
            "-hls_playlist_type", "vod",
            "-hls_segment_filename", &format!("{}/segment_%03d.ts", hls_dir),
            "-y", &playlist_path,
        ])
        .output();

    match output {
        Ok(result) => {
            if result.status.success() {
                // get video info using ffprobe
                let probe_output = Command::new("ffprobe")
                    .args(&[
                        "-v", "quiet",
                        "-print_format", "json",
                        "-show_format",
                        "-show_streams",
                        &temp_path_clone,
                    ])
                    .output();

                let mut duration = None;
                let mut resolution = None;

                if let Ok(probe) = probe_output {
                    if let Ok(json_str) = String::from_utf8(probe.stdout) {
                        if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                            // get duration
                            if let Some(format) = json_val.get("format") {
                                if let Some(dur) = format.get("duration") {
                                    duration = dur.as_str().and_then(|s| s.parse::<f64>().ok());
                                }
                            }
                            // get resolution
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

                // create thumbnail
                let thumbnail_path = format!("videos/output/{}/thumbnail.jpg", id_clone);
                let mut thumbnail_generated = false;

                if let Some(dur) = duration {
                    // pick random time in video
                    let random_second = dur * (0.1 + (rand::random::<f64>() * 0.8));
                    println!("creating thumbnail at {} seconds", random_second);

                    let thumbnail_output = Command::new("ffmpeg")
                        .args(&[
                            "-ss", &format!("{}", random_second),
                            "-i", &temp_path_clone,
                            "-vframes", "1",
                            "-vf", "scale=1280:-1",
                            "-q:v", "2",
                            "-y", &thumbnail_path,
                        ])
                        .output();

                    match thumbnail_output {
                        Ok(result) if result.status.success() => {
                            println!("thumbnail created");
                            thumbnail_generated = true;
                        }
                        Ok(result) => {
                            let error = String::from_utf8_lossy(&result.stderr);
                            println!("thumbnail failed: {}", error);
                        }
                        Err(e) => {
                            println!("thumbnail command failed: {}", e);
                        }
                    }
                }

                // save to database
                let thumbnail_filename = if thumbnail_generated {
                    Some(format!("{}/thumbnail.jpg", id_clone))
                } else {
                    None
                };

                // Update database with processing results
                if let Ok(conn) = db_clone.lock() {
                    let result = conn.execute(
                        "UPDATE videos SET duration = ?1, resolution = ?2, thumbnail = ?3 WHERE id = ?4",
                        params![
                            duration,
                            resolution,
                            thumbnail_filename,
                            id_clone.to_string()
                        ],
                    );

                    if let Err(e) = result {
                        println!("failed to update video in db: {}", e);
                    } else {
                        println!("video {} processing complete and saved", id_clone);
                    }
                }

                println!("background processing complete for video: {}", id_clone);
            } else {
                let error = String::from_utf8_lossy(&result.stderr);
                println!("ffmpeg error for video {}: {}", id_clone, error);
                // Clean up on error
                std::fs::remove_dir_all(format!("videos/input/{}", id_clone)).ok();
                std::fs::remove_dir_all(format!("videos/output/{}", id_clone)).ok();

                // Remove failed video from database
                if let Ok(conn) = db_clone.lock() {
                    conn.execute(
                        "DELETE FROM videos WHERE id = ?1",
                        params![id_clone.to_string()],
                    ).ok();
                }
            }
        }
        Err(e) => {
            println!("ffmpeg command failed for video {}: {}", id_clone, e);
            // Clean up on error
            std::fs::remove_dir_all(format!("videos/input/{}", id_clone)).ok();
            std::fs::remove_dir_all(format!("videos/output/{}", id_clone)).ok();

            // Remove failed video from database
            if let Ok(conn) = db_clone.lock() {
                conn.execute(
                    "DELETE FROM videos WHERE id = ?1",
                    params![id_clone.to_string()],
                ).ok();
            }
        }
    }
    });

    // Return immediate success - processing happens in background
    return (
        StatusCode::OK,
        format!("Upload successful! Video processing started. id: {}", id),
    );
}