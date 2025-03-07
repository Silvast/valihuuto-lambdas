use rss::{Channel, Item};
use std::error::Error;
use reqwest;
use urlencoding::encode;
use serde_json::Value;
use regex::Regex;
use chrono::{DateTime, FixedOffset};
use aws_sdk_sqs::Client as SqsClient;
use serde_json::json;
use dotenv::dotenv;

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

pub async fn fetch_memo(url: &str) -> String {
    let response = reqwest::get(url).await.unwrap();
    let body = response.text().await.unwrap();
    body
}

pub async fn get_feed(date: DateTime<FixedOffset>) -> Result<Vec<Item>, Box<dyn Error>> {
    println!("RSS_URL: {:?}", std::env::var("RSS_URL").unwrap());
    println!("QUEUE_URL: {:?}", std::env::var("QUEUE_URL").unwrap());
    println!("DYNAMODB_TABLE: {:?}", std::env::var("DYNAMODB_TABLE").unwrap());
    println!("EDUSKUNTA_DB_URL: {:?}", std::env::var("EDUSKUNTA_DB_URL").unwrap());
    
    let content = reqwest::get("https://www.eduskunta.fi/_layouts/15/feed.aspx?xsl=1&web=%2FFI%2Frss%2Dfeeds&page=8192fae7-172f-46ba-8605-75c1e750007a&wp=3527e156-7a72-443c-8698-9b5596317471&pageurl=%2FFI%2Frss%2Dfeeds%2FSivut%2FTaysistuntojen%2Dpoytakirjat%2DRSS%2Easpx")
        .await?
        .bytes()
        .await?;
    let channel = Channel::read_from(&content[..])?;
    
    // Filter items based on publication date
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
    // Get AWS config and create SQS client
    let config = aws_config::load_from_env().await;
    let sqs_client = SqsClient::new(&config);
    let queue_url = "https://sqs.eu-west-1.amazonaws.com/401704393864/Valihuuto.fifo";
    
    // Add detailed error messages
    println!("Attempting to send messages to SQS queue: {}", queue_url);
    
    // 1. Send memo data as first message with title, link and description
    let memo_message = json!({
        "title": item.title().unwrap_or_default(),
        "link": item.link().unwrap_or_default(),
        "description": item.description().unwrap_or_default()
    }).to_string();
    
    let memo_dedup_id =  String::from(chrono::Utc::now().timestamp().to_string());

    
    match sqs_client.send_message()
        .queue_url(queue_url)
        .message_body(memo_message)
        .message_group_id("memo_group")
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
    
    // 2. Send each shout as its own message
    for (i, shout) in shout_data.iter().enumerate() {
        let shout_dedup_id = String::from("shout_") + &memo_dedup_id + "_" + &i.to_string();
        println!("Shout dedup ID: {}", shout_dedup_id);
        
        match sqs_client.send_message()
            .queue_url(queue_url)
            .message_body(shout)
            .message_group_id("shout_group")
            .message_deduplication_id(shout_dedup_id)
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
    let eduskunta_db_url = std::env::var("EDUSKUNTA_DB_URL").unwrap();
    println!("EDUSKUNTA_DB_URL: {}", eduskunta_db_url);
    let prefix = "https://avoindata.eduskunta.fi/api/v1/tables/VaskiData/rows?perPage=10&page=0&columnName=Eduskuntatunnus&columnValue=";
    let suffix = encode(item.title().unwrap_or_default()).to_string();

    let url = format!("{}{}", prefix, suffix);
    // println!("URL: {}", url);

    let memo = Memo {
        title: item.title().unwrap_or_default().to_string(),
        link: item.link().unwrap_or_default().to_string(),
        description: item.description().unwrap_or_default().to_string(),
        content: String::from(fetch_memo(&url).await),
    };

    let json_data: Value = match serde_json::from_str(&memo.content) {
        Ok(data) => data,
        Err(err) => {
            //println!("Error: {}", err);
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
    
    // Send data to SQS - uncomment this line
    send_to_sqs(item, &shout_data).await?;
    
    Ok(())
}