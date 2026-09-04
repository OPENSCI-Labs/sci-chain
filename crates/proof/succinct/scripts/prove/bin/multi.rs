//! Multi-block range proof generation binary.
#![recursion_limit = "256"]

use std::{
    env, fs,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use base_proof_succinct_host_utils::{
    block_range::get_validated_block_range,
    fetcher::OPSuccinctDataFetcher,
    host::OPSuccinctHost,
    network::{build_network_prover_from_env, parse_fulfillment_strategy},
    proof_cache::save_range_proof,
    stats::ExecutionStats,
    witness_cache::{load_stdin_from_cache, save_stdin_to_cache},
    witness_generation::WitnessGenerator,
};
use base_proof_succinct_proof_utils::{
    cluster_range_proof, get_range_elf_embedded, initialize_host, is_cluster_mode,
};
use base_proof_succinct_prove::execute_multi;
use base_proof_succinct_scripts::HostExecutorArgs;
use clap::Parser;
#[cfg(feature = "cuda")]
use sp1_sdk::ProverClient;
use sp1_sdk::{
    Elf, ProveRequest, Prover,
    blocking::{CpuProver, ProveRequest as BlockingProveRequest, Prover as BlockingProver},
    utils,
};
use tracing::{debug, info, warn};

/// Which final proof artifact to produce, selected by the `PROOF_MODE` env var.
#[derive(Debug, Clone, Copy)]
enum ProofMode {
    Compressed,
    Groth16,
    Plonk,
}

impl ProofMode {
    /// Reads `PROOF_MODE` (default `compressed`); errors on an unknown value.
    fn from_env() -> Result<Self> {
        match env::var("PROOF_MODE").unwrap_or_else(|_| "compressed".to_string()).as_str() {
            "compressed" => Ok(Self::Compressed),
            "groth16" => Ok(Self::Groth16),
            "plonk" => Ok(Self::Plonk),
            other => {
                anyhow::bail!("unknown PROOF_MODE={other:?}; expected compressed|groth16|plonk")
            }
        }
    }
}

/// Execute the Succinct program for multiple blocks.
#[tokio::main]
async fn main() -> Result<()> {
    let args = HostExecutorArgs::parse();

    dotenv::from_path(&args.env_file)
        .context(format!("Environment file not found: {}", args.env_file.display()))?;
    utils::setup_logger();

    let data_fetcher = OPSuccinctDataFetcher::new_with_rollup_config().await?;

    let host = initialize_host(Arc::new(data_fetcher.clone()));

    // If the end block is provided, check that it is less than the latest finalized block. If the
    // end block is not provided, use the latest finalized block.
    let (l2_start_block, l2_end_block) = get_validated_block_range(
        host.as_ref(),
        &data_fetcher,
        args.start,
        args.end,
        args.default_range,
    )
    .await?;

    let l2_chain_id = data_fetcher.get_l2_chain_id().await?;

    // Helper closure to generate stdin (runs witness generation and converts to SP1Stdin)
    let generate_stdin = || async {
        let host_args = host
            .fetch(
                l2_start_block,
                l2_end_block,
                None,
                base_proof_succinct_client_utils::client::DEFAULT_INTERMEDIATE_ROOT_INTERVAL,
                args.safe_db_fallback,
            )
            .await?;
        debug!("Host args: {:?}", host_args);

        let start_time = Instant::now();
        let witness = host.run(&host_args).await?;
        let duration = start_time.elapsed();

        // Convert witness to SP1Stdin
        let stdin = host.witness_generator().get_sp1_stdin(witness)?;

        // Save to cache if enabled
        if args.cache {
            let cache_path =
                save_stdin_to_cache(l2_chain_id, l2_start_block, l2_end_block, &stdin)?;
            info!("Saved stdin to cache: {}", cache_path.display());
        }

        Ok::<_, anyhow::Error>((stdin, duration))
    };

    // Check cache first if enabled (with graceful fallback)
    let (sp1_stdin, witness_generation_duration) = if args.cache {
        match load_stdin_from_cache(l2_chain_id, l2_start_block, l2_end_block) {
            Ok(Some(stdin)) => {
                info!("Loaded stdin from cache");
                (stdin, Duration::ZERO)
            }
            Ok(None) => generate_stdin().await?,
            Err(e) => {
                warn!("Failed to load cache: {e}, regenerating...");
                generate_stdin().await?
            }
        }
    } else {
        generate_stdin().await?
    };

    if args.prove {
        if is_cluster_mode() {
            let proof = cluster_range_proof(args.cluster_timeout, sp1_stdin).await?;
            let path = save_range_proof(l2_chain_id, l2_start_block, l2_end_block, &proof)?;
            info!("Range proof saved to {}", path.display());
        } else if env::var("SP1_PROVER").as_deref() == Ok("cpu") {
            // Local CPU proof — no prover network/cluster required (SP1_PROVER=cpu).
            // CpuProver is synchronous and CPU-bound, so run it on a blocking thread to keep
            // the async runtime responsive; errors propagate through the JoinHandle via await??.
            let proof_mode = ProofMode::from_env()?;
            info!(?proof_mode, "Generating local CPU proof (no prover network)");
            let prove_start = Instant::now();
            let proof = tokio::task::spawn_blocking(move || -> Result<_> {
                let prover = CpuProver::new();
                let pk = BlockingProver::setup(&prover, Elf::Static(get_range_elf_embedded()))
                    .map_err(|e| anyhow::anyhow!("range ELF setup failed: {e:?}"))?;
                let req = BlockingProver::prove(&prover, &pk, sp1_stdin);
                let req = match proof_mode {
                    ProofMode::Compressed => BlockingProveRequest::compressed(req),
                    ProofMode::Groth16 => BlockingProveRequest::groth16(req),
                    ProofMode::Plonk => BlockingProveRequest::plonk(req),
                };
                BlockingProveRequest::run(req)
                    .map_err(|e| anyhow::anyhow!("CPU proving failed: {e:?}"))
            })
            .await??;
            info!(elapsed = ?prove_start.elapsed(), ?proof_mode, "CPU proof generated");
            let path = save_range_proof(l2_chain_id, l2_start_block, l2_end_block, &proof)?;
            info!("CPU range proof saved to {}", path.display());
        } else if env::var("SP1_PROVER").as_deref() == Ok("cuda") {
            // Local NVIDIA GPU proof via SP1's CUDA prover (talks to the sp1-gpu docker
            // container). Async, not CPU-bound, so it runs directly on the runtime. Requires
            // building with `--features cuda`; the `.cuda()` builder is gated on sp1-sdk/cuda.
            #[cfg(not(feature = "cuda"))]
            anyhow::bail!(
                "SP1_PROVER=cuda requires building `multi` with `--features cuda` (enables \
                 sp1-sdk/cuda). Rebuild: cargo build --release -p base-proof-succinct-prove \
                 --bin multi --features cuda"
            );
            #[cfg(feature = "cuda")]
            {
                let proof_mode = ProofMode::from_env()?;
                info!(?proof_mode, "Generating CUDA proof on local NVIDIA GPU");
                let prove_start = Instant::now();
                let prover = ProverClient::builder().cuda().build().await;
                let pk = prover.setup(Elf::Static(get_range_elf_embedded())).await?;
                let proof = match proof_mode {
                    ProofMode::Compressed => prover.prove(&pk, sp1_stdin).compressed().await,
                    ProofMode::Groth16 => prover.prove(&pk, sp1_stdin).groth16().await,
                    ProofMode::Plonk => prover.prove(&pk, sp1_stdin).plonk().await,
                }
                .map_err(|e| anyhow::anyhow!("CUDA proving failed: {e:?}"))?;
                info!(elapsed = ?prove_start.elapsed(), ?proof_mode, "CUDA proof generated");
                let path = save_range_proof(l2_chain_id, l2_start_block, l2_end_block, &proof)?;
                info!("CUDA range proof saved to {}", path.display());
            }
        } else {
            let range_proof_strategy = parse_fulfillment_strategy(
                env::var("RANGE_PROOF_STRATEGY").unwrap_or_else(|_| "reserved".to_string()),
            )?;
            let prover = build_network_prover_from_env(range_proof_strategy).await?;
            let pk = prover.setup(Elf::Static(get_range_elf_embedded())).await?;
            let proof = prover
                .prove(&pk, sp1_stdin)
                .compressed()
                .strategy(range_proof_strategy)
                .await
                .expect("proving failed");
            let path = save_range_proof(l2_chain_id, l2_start_block, l2_end_block, &proof)?;
            info!("Range proof saved to {}", path.display());
        }
    } else {
        let (block_data, report, execution_duration) =
            execute_multi(&data_fetcher, sp1_stdin, l2_start_block, l2_end_block).await?;

        let stats = ExecutionStats::new(
            0,
            &block_data,
            &report,
            witness_generation_duration.as_secs(),
            execution_duration.as_secs(),
        );

        println!("Execution Stats: \n{stats:?}");

        // Create the report directory if it doesn't exist.
        let report_dir = format!("execution-reports/multi/{l2_chain_id}");
        if !std::path::Path::new(&report_dir).exists() {
            fs::create_dir_all(&report_dir)?;
        }

        let report_path =
            format!("execution-reports/multi/{l2_chain_id}/{l2_start_block}-{l2_end_block}.csv");

        // Write to CSV.
        let mut csv_writer = csv::Writer::from_path(report_path)?;
        csv_writer.serialize(&stats)?;
        csv_writer.flush()?;
    }

    Ok(())
}
