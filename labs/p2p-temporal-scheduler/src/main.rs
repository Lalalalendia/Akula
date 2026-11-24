use akula_p2p_temporal_scheduler_lab::{bridge_demo, schedule, Strategy};

fn main() -> Result<(), String> {
    let scenario = bridge_demo();
    println!("Synthetic 4-peer bridge: 2 unique demands, 1 chunk, 4 slots.");
    for (name, strategy) in [
        ("S3-only", Strategy::S3Only),
        ("Greedy on-demand", Strategy::GreedyOnDemand),
        ("Temporal route", Strategy::TemporalRoute),
    ] {
        let result = schedule(&scenario, strategy)?;
        println!(
            "{name}: completed={}/{}, s3_chunks={}, peer_chunks={}, last_completion={:?}",
            result.completed,
            result.total,
            result.s3_chunks,
            result.peer_chunks,
            result.last_completion
        );
        for transfer in result.transfers {
            println!("  {transfer:?}");
        }
    }
    Ok(())
}
