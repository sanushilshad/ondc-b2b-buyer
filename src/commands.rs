use crate::utils::generate_user_token;
use crate::{kafka_client, migration};
#[tracing::instrument(name = "Run custom command")]
pub async fn run_custom_commands(args: Vec<String>) -> Result<(), anyhow::Error> {
    if args.len() < 2 {
        eprintln!("Invalid command. Please provide a valid command.");
        return Ok(());
    }
    let command = args[1].as_str();

    match command {
        "migrate" => {
            migration::run_migrations().await;
        }
        "sqlx_migrate" => {
            migration::migrate_using_sqlx().await;
        }
        "generate_service_token" => {
            // let arg = args.get(2).unwrap_or(&TopicType::Search.to_string());
            generate_user_token().await;
        }
        "generate_kafka_topic" => {
            // let arg = args.get(2).unwrap_or(&TopicType::Search.to_string());
            kafka_client::create_kafka_topic_command().await;
        }
        _ => {
            eprintln!("Unknown command: {}. Please use a valid command.", command);
        }
    }

    Ok(())
}
