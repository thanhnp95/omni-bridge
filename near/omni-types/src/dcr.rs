use near_sdk::json_types::U128;
use near_sdk::serde::{Serialize, Deserialize};
use near_sdk::near;

/// Same format as BTC — txid:vout
pub type OutPoint = String;

/// Message sent from NEAR → DCR connector
#[derive(Debug, Serialize, Deserialize)]
pub enum DcrTokenReceiverMessage {
    DepositProtocolFee,

    Withdraw {
        target_dcr_address: String,

        /// UTXOs being spent
        input: Vec<OutPoint>,

        /// Outputs of the DCR tx
        output: Vec<DcrTxOut>,

        /// atoms per kB (DCR fee model)
        max_fee_rate: Option<U128>,
    },
}

/// Decred output format
#[near(serializers=[json])]
#[derive(Debug, Clone)]
pub struct DcrTxOut {
    /// amount in atoms
    pub value: u64,

    /// script version
    pub version: u16,

    /// script hex
    pub pk_script: String,
}
