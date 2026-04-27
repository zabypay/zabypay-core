use serde::{Deserialize, Serialize};
use std::fmt;

/// Withdrawal fund state enum - tracks the state of funds in the withdrawal process
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WithdrawalFundsState {
    /// Funds are unlocked and available for use
    Unlocked,
    /// Funds are locked for this withdrawal (tx created/broadcasted)
    Locked,
    /// Funds have been confirmed as transferred (finality reached)
    Confirmed,
    /// Funds have been released back to available (tx failed/dropped)
    Released,
}

impl fmt::Display for WithdrawalFundsState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WithdrawalFundsState::Unlocked => write!(f, "unlocked"),
            WithdrawalFundsState::Locked => write!(f, "locked"),
            WithdrawalFundsState::Confirmed => write!(f, "confirmed"),
            WithdrawalFundsState::Released => write!(f, "released"),
        }
    }
}

impl std::str::FromStr for WithdrawalFundsState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "unlocked" => Ok(WithdrawalFundsState::Unlocked),
            "locked" => Ok(WithdrawalFundsState::Locked),
            "confirmed" => Ok(WithdrawalFundsState::Confirmed),
            "released" => Ok(WithdrawalFundsState::Released),
            _ => Err(format!("Invalid withdrawal funds state: {}", s)),
        }
    }
}

/// Withdrawal lifecycle state enum - tracks the overall lifecycle of a withdrawal
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WithdrawalLifecycleState {
    /// Withdrawal quote has been created but not yet broadcasted
    Quoted,
    /// Transaction has been broadcasted to the network
    Broadcasted,
    /// Transaction is processing (waiting for confirmations)
    Processing,
    /// Transaction has been confirmed (finality reached)
    Confirmed,
    /// Transaction has failed or been rejected
    Failed,
    /// Withdrawal has been cancelled by user or system
    Cancelled,
}

impl fmt::Display for WithdrawalLifecycleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WithdrawalLifecycleState::Quoted => write!(f, "quoted"),
            WithdrawalLifecycleState::Broadcasted => write!(f, "broadcasted"),
            WithdrawalLifecycleState::Processing => write!(f, "processing"),
            WithdrawalLifecycleState::Confirmed => write!(f, "confirmed"),
            WithdrawalLifecycleState::Failed => write!(f, "failed"),
            WithdrawalLifecycleState::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::str::FromStr for WithdrawalLifecycleState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "quoted" => Ok(WithdrawalLifecycleState::Quoted),
            "broadcasted" => Ok(WithdrawalLifecycleState::Broadcasted),
            "processing" => Ok(WithdrawalLifecycleState::Processing),
            "confirmed" => Ok(WithdrawalLifecycleState::Confirmed),
            "failed" => Ok(WithdrawalLifecycleState::Failed),
            "cancelled" => Ok(WithdrawalLifecycleState::Cancelled),
            _ => Err(format!("Invalid withdrawal lifecycle state: {}", s)),
        }
    }
}

impl WithdrawalFundsState {
    /// Check if funds are currently locked (unavailable for other withdrawals)
    pub fn is_locked(&self) -> bool {
        matches!(self, WithdrawalFundsState::Locked)
    }

    /// Check if funds have been permanently committed (confirmed withdrawal)
    pub fn is_committed(&self) -> bool {
        matches!(self, WithdrawalFundsState::Confirmed)
    }

    /// Check if funds are available (unlocked or released)
    pub fn is_available(&self) -> bool {
        matches!(
            self,
            WithdrawalFundsState::Unlocked | WithdrawalFundsState::Released
        )
    }
}

impl WithdrawalLifecycleState {
    /// Check if withdrawal is in a terminal state (cannot change)
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            WithdrawalLifecycleState::Confirmed
                | WithdrawalLifecycleState::Failed
                | WithdrawalLifecycleState::Cancelled
        )
    }

    /// Check if withdrawal is actively being processed
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            WithdrawalLifecycleState::Broadcasted | WithdrawalLifecycleState::Processing
        )
    }

    /// Check if withdrawal can be cancelled
    pub fn can_be_cancelled(&self) -> bool {
        matches!(
            self,
            WithdrawalLifecycleState::Quoted | WithdrawalLifecycleState::Broadcasted
        )
    }

    /// Get the expected funds state for this lifecycle state
    pub fn expected_funds_state(&self) -> WithdrawalFundsState {
        match self {
            WithdrawalLifecycleState::Quoted => WithdrawalFundsState::Unlocked,
            WithdrawalLifecycleState::Broadcasted | WithdrawalLifecycleState::Processing => {
                WithdrawalFundsState::Locked
            }
            WithdrawalLifecycleState::Confirmed => WithdrawalFundsState::Confirmed,
            WithdrawalLifecycleState::Failed | WithdrawalLifecycleState::Cancelled => {
                WithdrawalFundsState::Released
            }
        }
    }
}

/// Configuration for chain-specific withdrawal monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainConfig {
    pub id: String,
    pub network: String,
    pub environment: String,
    pub required_confirmations: i32,
    pub pending_ttl_seconds: i32,
    pub reorg_grace_confirmations: i32,
    pub poll_interval_ms: i32,
    pub rpc_url: Option<String>,
    pub explorer_url_template: Option<String>,
    pub enabled: bool,
}

impl ChainConfig {
    /// Get explorer URL for a specific transaction
    pub fn get_explorer_url(&self, tx_hash: &str) -> Option<String> {
        self.explorer_url_template
            .as_ref()
            .map(|template| template.replace("{tx_hash}", tx_hash))
    }

    /// Check if this configuration supports the given network and environment
    pub fn matches(&self, network: &str, environment: &str) -> bool {
        self.network == network && self.environment == environment && self.enabled
    }
}
