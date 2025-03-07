use lambda_runtime::{tracing, Error, LambdaEvent};
use aws_lambda_events::event::eventbridge::EventBridgeEvent;
use chrono::{DateTime, FixedOffset, Utc, TimeZone, Duration};
use aws_sdk_dynamodb::Client as DynamoClient;
use aws_config;
use dotenv::dotenv;


pub(crate)async fn function_handler(event: LambdaEvent<EventBridgeEvent>) -> Result<(), Error> {
    println!("Received event: {:?}", event);
    Ok(())
}


