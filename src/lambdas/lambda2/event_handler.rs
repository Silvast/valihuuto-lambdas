use lambda_runtime::{tracing, Error, LambdaEvent};
use aws_lambda_events::event::eventbridge::EventBridgeEvent;
use aws_config;
use aws_sdk_sqs::{Client as SqsClient, types::Message};
use dotenv::dotenv;
use serde_json::Value;
use std::env;


use crate::social_media::{MessageType, send_to_bluesky};

pub(crate) async fn function_handler(_event: LambdaEvent<EventBridgeEvent>) -> Result<(), Error> {
    dotenv().ok();

    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let client = SqsClient::new(&config);    
    let queue_url = env::var("QUEUE_URL").expect("QUEUE_URL must be set in .env file");
    
    let receive_result = client
        .receive_message()
        .queue_url(&queue_url)
        .max_number_of_messages(1)
        .send()
        .await?;
    
    // Check if there is a message
    if let Some(messages) = receive_result.messages {
        if !messages.is_empty() {
            let message = &messages[0];
            
            if let Err(e) = process_message(message).await {
                println!("Error processing message: {:?}", e);
                return Err(e.into());
            }
            
        
            client
                .delete_message()
                .queue_url(&queue_url)
                .receipt_handle(message.receipt_handle.as_ref().unwrap())
                .send()
                .await?;
        } else {
            println!("No messages in the queue");
        }
    } else {
        println!("No messages in the queue");
    }
    
    Ok(())
}

async fn process_message(message: &Message) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(body) = &message.body {
       
        let message_type = match serde_json::from_str::<Value>(body) {
            Ok(parsed) if parsed.get("link").is_some() => {
                // If it has a link field, it's a memo
                let title = parsed.get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or_default()
                    .to_string();
                
                let link = parsed.get("link")
                    .and_then(|l| l.as_str())
                    .unwrap_or_default()
                    .to_string();
                
                MessageType::Memo { title, link }
            },
            _ => {

                MessageType::Shout { content: body.clone() }
            }
        };
        
        
        send_to_bluesky(&message_type).await?;
    }
    
    Ok(())
}


