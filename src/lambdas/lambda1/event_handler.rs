use lambda_runtime::{tracing, Error, LambdaEvent};
use aws_lambda_events::event::eventbridge::EventBridgeEvent;
use crate::memos::{find_text_between_brackets, get_feed, edit_memo};
use chrono::{DateTime, FixedOffset, Utc, TimeZone, Duration};
use aws_sdk_dynamodb::Client as DynamoClient;
use aws_config;
use dotenv::dotenv;



async fn update_last_checked_date(client: &DynamoClient, date: DateTime<FixedOffset>) -> Result<(), Error> {
    // Convert the DateTime to RFC3339 string format
    let date_str = date.to_rfc3339();
    println!("Updating lastchecked to: {}", date_str);
    
    // Update the item in DynamoDB
    let result = client
        .update_item()
        .table_name("Valihuuto")
        .key("id", aws_sdk_dynamodb::types::AttributeValue::N("1".to_string()))
        .update_expression("SET lastchecked = :date")
        .expression_attribute_values(
            ":date", 
            aws_sdk_dynamodb::types::AttributeValue::S(date_str)
        )
        .send()
        .await?;
        
    println!("Successfully updated lastchecked date in DynamoDB");
    Ok(())
}

async fn get_last_checked_date(client: &DynamoClient) -> Result<DateTime<FixedOffset>, Error> {
    let result = client
        .get_item()
        .table_name("Valihuuto")
        .key("id", aws_sdk_dynamodb::types::AttributeValue::N("1".to_string()))
        .send()
        .await?;

    if let Some(item) = result.item {
        println!("Item: {:?}", item.get("lastchecked").unwrap().as_s().unwrap());
        Ok(DateTime::parse_from_rfc3339(item.get("lastchecked").unwrap().as_s().unwrap())?)
    } else {
        // If no item was found, return today's date minus two days
        let yesterday = Utc::now().checked_sub_signed(Duration::days(1))
            .expect("Date calculation should not fail");
        Ok(DateTime::parse_from_rfc3339(&yesterday.to_rfc3339())?)
    }
}

pub(crate)async fn function_handler(event: LambdaEvent<EventBridgeEvent>) -> Result<(), Error> {

    let client = DynamoClient::new(&aws_config::load_from_env().await);
    let date = get_last_checked_date(&client).await?;
    let items = get_feed(date).await.map_err(|e| Error::from(format!("Feed error: {}", e)))?;
    
    for item in &items {
        edit_memo(item).await.map_err(|e| Error::from(format!("Edit memo error: {}", e)))?;
    }
    
    // Convert Utc::now() to DateTime<FixedOffset> using with_timezone
    let current_time = Utc::now();
    let fixed_offset_time = current_time.with_timezone(&FixedOffset::east_opt(0).unwrap());
    
    update_last_checked_date(&client, fixed_offset_time).await.map_err(|e| Error::from(format!("Update last checked date error: {}", e)))?;
    
    println!("last checked date: {:?}", date);
    println!("rss items: {:?}", items);
    let payload = event.payload;
    tracing::info!("Payload: {:?}", payload);   

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lambda_runtime::{Context, LambdaEvent};

    #[test]
    fn test_event_handler() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let event = LambdaEvent::new(EventBridgeEvent::default(), Context::default());
            let response = function_handler(event).await.unwrap();
            assert_eq!((), response);
        });
    }
}
