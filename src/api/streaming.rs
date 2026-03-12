use axum::{
    extract::Path,
    http::{StatusCode, header},
    response::IntoResponse,
};
use std::fs::{read_to_string, read};

// serve m3u8 playlist for hls streaming
pub async fn serve_playlist(Path(video_id): Path<String>) -> impl IntoResponse {
    let playlist_path = format!("videos/output/{}/playlist.m3u8", video_id);

    match read_to_string(&playlist_path) {
        Ok(content) => {
            // fix paths in playlist to use our api endpoints
            let updated_content = content
                .lines()
                .map(|line| {
                    if line.ends_with(".ts") {
                        // convert segment file to api path
                        format!("/api/stream/{}/{}", video_id, line)
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
            "playlist not found".to_string(),
        ),
    }
}

// serve individual video segments
pub async fn serve_segment(Path((video_id, segment)): Path<(String, String)>) -> impl IntoResponse {
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
                "segment not found",
            ).into_response()
        }
    }
}