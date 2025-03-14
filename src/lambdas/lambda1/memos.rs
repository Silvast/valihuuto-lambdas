use rss::{Channel, Item};
use std::error::Error;
use reqwest;
use urlencoding::encode;
use serde_json::Value;
use regex::Regex;
use chrono::{DateTime, FixedOffset};
use aws_sdk_sqs::Client as SqsClient;
use aws_config::BehaviorVersion;
use serde_json::json;
use dotenv::dotenv;
use reqwest::header::{HeaderMap, USER_AGENT};



#[derive(Debug)]
pub struct Memo
{
    title: String,
    link: String,
    description: String,
    content: String
}

pub fn find_text_between_brackets(input: &str) -> Vec<String> {
    let re = Regex::new(r"\[([^\[\]]+)\]").unwrap();
    let mut results = Vec::new();

    for cap in re.captures_iter(input) {
        if let Some(matched) = cap.get(1) {
            results.push(matched.as_str().to_string());
        }
    }
    results
}

pub async fn fetch_memo(url: &str) -> Result<String, Box<dyn Error>> {
    println!("Fetching memo from URL: {}", url);

    // Create a custom User-Agent string
    let custom_user_agent = "MyCustomUserAgent/1.0";

    // Create a client and set headers properly
    let client = reqwest::Client::new();
    let response = match client.get(url)
        .header(USER_AGENT, custom_user_agent)
        .send()
        .await {
        Ok(resp) => {
            if !resp.status().is_success() {
                return Err(format!("API returned error status: {}", resp.status()).into());
            }
            resp
        },
        Err(e) => {
            println!("Error fetching memo: {}", e);
            return Err(Box::new(e));
        }
    };
    
    let body = match response.text().await {
        Ok(text) => {
            println!("Response body length: {} bytes", text.len());
            if text.trim().is_empty() {
                return Err("API returned empty response".into());
            }
            text
        },
        Err(e) => {
            println!("Error parsing response body: {}", e);
            return Err(Box::new(e));
        }
    };
    
    Ok(body)
}

pub async fn get_feed(date: DateTime<FixedOffset>) -> Result<Vec<Item>, Box<dyn Error>> {

    let rss_url = std::env::var("RSS_URL").unwrap();
    let content = reqwest::get(&rss_url)
        .await?
        .bytes()
        .await?;
    let channel = Channel::read_from(&content[..])?;
    
    
    let filtered_items: Vec<Item> = channel
        .items()
        .iter()
        .filter(|item| {
            if let Some(pub_date) = item.pub_date() {
                // Parse the RSS pub_date string into DateTime
                if let Ok(item_date) = DateTime::parse_from_rfc2822(pub_date) {
                    return item_date > date;
                }
            }
            false
        })
        .cloned()
        .collect();

    Ok(filtered_items)
}

pub async fn send_to_sqs(item: &Item, shout_data: &[String]) -> Result<(), Box<dyn Error>> {
    
    let config = aws_config::defaults(BehaviorVersion::latest()).load().await;
    let sqs_client = SqsClient::new(&config);
    let queue_url = std::env::var("QUEUE_URL").unwrap();
    
    println!("Attempting to send messages to SQS queue: {}", queue_url);
    
    let memo_message = json!({
        "title": item.title().unwrap_or_default(),
        "link": item.link().unwrap_or_default(),
        "description": item.description().unwrap_or_default()
    }).to_string();
    
    let timestamp = chrono::Utc::now().timestamp_millis().to_string();
    let memo_dedup_id = format!("memo_dedup_{}", timestamp);
    
    match sqs_client.send_message()
        .queue_url(&queue_url)
        .message_body(memo_message)
        .message_group_id("memo_shout_group")
        .message_deduplication_id(&memo_dedup_id)
        .send()
        .await {
            Ok(_) => println!("Successfully sent memo data to SQS: {}", item.title().unwrap_or_default()),
            Err(e) => {
                println!("Failed to send memo to SQS: {:?}", e);
                return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, 
                                   format!("SQS Send Error: {}", e))));
            }
        }
    
    for (i, shout) in shout_data.iter().enumerate() {
        let shout_dedup_id = format!("shout_dedup_{}_{}", timestamp, i);
        println!("Shout dedup ID: {}", shout_dedup_id);
        
        match sqs_client.send_message()
            .queue_url(&queue_url)
            .message_body(shout)
            .message_group_id("memo_shout_group")
            .message_deduplication_id(&shout_dedup_id)
            .send()
            .await {
                Ok(_) => println!("Successfully sent shout to SQS: {}", shout),
                Err(e) => {
                    println!("Failed to send shout to SQS: {:?}", e);
                    return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, 
                                       format!("SQS Send Error: {}", e))));
                }
            }
    }
    
    Ok(())
}
    
pub async fn edit_memo(item: &Item) -> Result<(), Box<dyn Error>> {
    dotenv().ok();

    let eduskunta_db_url = std::env::var("EDUSKUNTA_DB_URL")
        .map_err(|e| format!("Environment variable EDUSKUNTA_DB_URL not set: {}", e))?;

    let suffix = encode(item.title().unwrap_or_default()).to_string();
    let url = format!("{}{}", eduskunta_db_url, suffix);
    println!("Full URL: {}", url);

   
    let content = match fetch_memo(&url).await {
        Ok(content) => content,
        Err(e) => {
            println!("Failed to fetch memo: {}", e);
            return Err(format!("Failed to fetch memo: {}", e).into());
        }
    };

    let memo = Memo {
        title: item.title().unwrap_or_default().to_string(),
        link: item.link().unwrap_or_default().to_string(),
        description: item.description().unwrap_or_default().to_string(),
        content: content,
    };

    let json_data: Value = match serde_json::from_str(&memo.content) {
        Ok(data) => data,
        Err(err) => {
            println!("JSON parsing error: {}. Content starts with: '{}'", 
                    err, &memo.content.chars().take(100).collect::<String>());
            return Err(Box::new(err));
        }
    };
    let mut shout_data: Vec<String> = Vec::<String>::new();

    if let Some(rows) = json_data["rowData"].as_array() {
        for row in rows {
          
            let shouts: Vec<String> = find_text_between_brackets(&String::from(row[1].as_str().unwrap()));
            shout_data.extend(shouts.into_iter().filter(|x| x != "Puhemies koputtaa").collect::<Vec<String>>());

        }
    }

    println!("Shouts: {:?}", shout_data);
    
    if !shout_data.is_empty() {
        send_to_sqs(item, &shout_data).await?;
    } else {
        println!("No shouts found, skipping SQS message");
    }
    
    Ok(())
}