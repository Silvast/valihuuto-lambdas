use chrono::Utc;
use dotenv::dotenv;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::env;
use std::error::Error;

#[derive(Debug, Deserialize)]
struct SessionResponse {
    #[serde(rename = "accessJwt")]
    access_jwt: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum MessageType {
    Shout { content: String },
    Memo { title: String, link: String },
}

pub async fn send_to_bluesky(message: &MessageType) -> Result<(), Box<dyn Error + Send + Sync>> {
    dotenv().ok();
    
    // Get password from environment variable
    let password = env::var("BLUESKY_PASSWD").expect("BLUESKY_PASSWD must be set in .env file");
    
    // Step 1: Get bearer token
    let client = Client::new();
    
    let session_response = client
        .post("https://bsky.social/xrpc/com.atproto.server.createSession")
        .header("Content-Type", "application/json")
        .json(&json!({
            "identifier": "valihuutobot.bsky.social",
            "password": password
        }))
        .send()
        .await?
        .json::<SessionResponse>()
        .await?;
    
    // Step 2: Create post with different content based on message type
    let post_text = match message {
        MessageType::Shout { content } => content.clone(),
        MessageType::Memo { title, link } => format!("Huudan välihuudot pöytäkirjasta: {} - {}", title, link),
    };
    
    // Current time in UTC formatted as required by Bluesky
    let current_time = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    
    // Create post
    let post_response = client
        .post("https://bsky.social/xrpc/com.atproto.repo.createRecord")
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", session_response.access_jwt))
        .json(&json!({
            "repo": "valihuutobot.bsky.social",
            "collection": "app.bsky.feed.post",
            "record": {
                "text": post_text,
                "createdAt": current_time
            }
        }))
        .send()
        .await?;
    
    // Check response
    if post_response.status().is_success() {
        println!("Successfully posted to Bluesky");
        Ok(())
    } else {
        let error_text = post_response.text().await?;
        println!("Failed to post to Bluesky: {}", error_text);
        Err(error_text.into())
    }
}
