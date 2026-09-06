mod app;
mod config;
mod provisioning;
mod storage;
mod wifi;

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!(
        "echo_byte provisioning firmware {}",
        env!("CARGO_PKG_VERSION")
    );

    let executor: edge_executor::LocalExecutor = Default::default();
    edge_executor::block_on(executor.run(app::run()))
}
