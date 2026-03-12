use axum::response::IntoResponse;

// simple health check endpoint
pub async fn health_check() -> impl IntoResponse {
    "server is running"
}