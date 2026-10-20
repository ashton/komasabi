use crate::models::integration::IntegrationType;
use crate::models::job::{JobStatus, JobType};
use crate::models::library::Library;
use crate::providers::{
    kavita::provider::KavitaProvider, komga::provider::KomgaProvider, opds::provider::OpdsProvider,
};
use crate::repository::{
    integration as integration_repository, job as job_repository, library as library_repository,
};

/// Spawns a dedicated OS thread that runs the job to completion in the background,
/// independent of the UI's async runtime.
pub fn spawn(integration_id: String) {
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build sync-integration-libraries job runtime");

        runtime.block_on(run(integration_id));
    });
}

async fn run(integration_id: String) {
    let job = match job_repository::create(
        JobType::SyncIntegrationLibraries,
        JobStatus::Running,
        Some(integration_id.clone()),
    )
    .await
    {
        Ok(job) => job,
        Err(error) => {
            tracing::debug!("failed to create sync-integration-libraries job: {error}");
            return;
        }
    };

    match sync(&integration_id).await {
        Ok(()) => {
            let _ = job_repository::update_status(job.id, JobStatus::Completed, None).await;
        }
        Err(error) => {
            tracing::debug!("sync-integration-libraries job failed: {error}");
            let _ = job_repository::update_status(job.id, JobStatus::Failed, Some(error)).await;
        }
    }
}

async fn sync(integration_id: &str) -> Result<(), String> {
    let remote_libraries = remote_libraries(integration_id).await?;
    let existing_libraries = library_repository::all().await?;

    for library in remote_libraries {
        let already_exists = existing_libraries.iter().any(|existing| {
            existing.integration_id == library.integration_id
                && existing.external_id == library.external_id
        });

        if already_exists {
            continue;
        }

        library_repository::create(library).await?;
    }

    Ok(())
}

/// Lists the libraries the integration's backend currently exposes, adapted to
/// this app's model.
async fn remote_libraries(integration_id: &str) -> Result<Vec<Library>, String> {
    let integration = integration_repository::find(integration_id).await?;
    let url = integration
        .url
        .clone()
        .ok_or_else(|| "integration is missing a url".to_string())?;

    match integration.integration_type {
        IntegrationType::Kavita => {
            let api_key = integration
                .api_key
                .clone()
                .ok_or_else(|| "integration is missing an api key".to_string())?;

            let provider = KavitaProvider::authenticate(url, api_key).await?;
            provider.list_libraries(integration_id).await
        }
        IntegrationType::Opds => {
            OpdsProvider::connect(url, integration.username, integration.password)
                .list_libraries(integration_id)
                .await
        }
        IntegrationType::Komga => {
            KomgaProvider::connect(
                url,
                integration.api_key,
                integration.username,
                integration.password,
            )?
            .list_libraries(integration_id)
            .await
        }
        other => Err(format!(
            "{} integrations can't be synced yet",
            other.label()
        )),
    }
}
