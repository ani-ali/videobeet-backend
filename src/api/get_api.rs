use axum::{http::header, response::IntoResponse};
use std::fs::read;

pub async fn hello_world() -> impl IntoResponse {
    "Hello world"
}
