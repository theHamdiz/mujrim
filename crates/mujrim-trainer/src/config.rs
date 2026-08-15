//! Training configuration for the NNUE pipeline.

/// Configuration for self-play data generation.
#[derive(Debug, Clone)]
pub struct DatagenConfig {
    /// Number of games to play
    pub num_games: u64,
    /// Search depth per move
    pub depth: i32,
    /// Number of threads for parallel games
    pub threads: usize,
    /// Output file path for training data
    pub output_path: String,
    /// Random opening moves before fixed-depth play begins
    pub random_plies: u32,
    /// Minimum game length to record (filters very short games)
    pub min_game_length: u32,
    /// Adjudicate draw if |eval| < this for N plies
    pub draw_adjudication_cp: i32,
    /// Number of consecutive plies for draw adjudication
    pub draw_adjudication_plies: u32,
    /// Adjudicate win if |eval| > this
    pub win_adjudication_cp: i32,
    /// Optional external network path to use during self-play.
    /// If `None`, uses the embedded default network.
    pub network_path: Option<String>,
    /// Search parameter preset to use during self-play.
    /// Maps to `SearchParams::for_preset()` (e.g. "akimbo", "stockfish").
    pub search_preset: String,
    /// Output encoding: `text`, `plain`, or `binpack`.
    pub format: String,
}

impl Default for DatagenConfig {
    fn default() -> Self {
        Self {
            num_games: 10_000,
            depth: 8,
            threads: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            output_path: "data.bin".to_string(),
            random_plies: 8,
            min_game_length: 16,
            draw_adjudication_cp: 10,
            draw_adjudication_plies: 10,
            win_adjudication_cp: 1000,
            network_path: None,
            search_preset: "akimbo".to_string(),
            format: "text".to_string(),
        }
    }
}

/// Configuration for NNUE training.
#[derive(Debug, Clone)]
pub struct TrainingConfig {
    /// Path to training data file
    pub data_path: String,
    /// Output path for trained network weights
    pub output_path: String,
    /// Number of training epochs
    pub epochs: u32,
    /// Batch size
    pub batch_size: u32,
    /// Initial learning rate
    pub learning_rate: f64,
    /// LR schedule: "cosine", "step", "constant"
    pub lr_schedule: String,
    /// WDL weight (0.0 = score only, 1.0 = WDL only, 0.5 = balanced)
    pub wdl_weight: f64,
    /// Network architecture string (hidden layer sizes)
    pub architecture: String,
    /// Optional base network path to use for fine-tuning.
    /// If `None`, trains from scratch.
    pub base_network: Option<String>,
    /// Comma-separated mix weights matching `data_path` entries.
    pub mix_weights: String,
    /// Seed for the post-mix epoch shuffle.
    pub mix_seed: u64,
}

impl Default for TrainingConfig {
    fn default() -> Self {
        Self {
            data_path: "data.bin".to_string(),
            output_path: "net.bin".to_string(),
            epochs: 100,
            batch_size: 16384,
            learning_rate: 0.001,
            lr_schedule: "cosine".to_string(),
            wdl_weight: 0.5,
            architecture: "768->1024x2->1".to_string(),
            base_network: None,
            mix_weights: String::new(),
            mix_seed: 1,
        }
    }
}
