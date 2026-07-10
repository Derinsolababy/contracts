// Shared types for escrow service
use soroban_sdk::{contracttype, Address, String, Vec, Env, Map, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EscrowStatus {
    Created,
    Funded,
    Active,
    Disputed,
    Resolved,
    Released,
    Cancelled,
    Expired,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DisputeStatus {
    Open,
    UnderReview,
    Resolved,
    Escalated,
    Expired,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DisputeResolution {
    ReleaseToBuyer,
    ReleaseToSeller,
    Split { buyer_amount: i128, seller_amount: i128 },
    Refund,
    PartialRelease { buyer_amount: i128, seller_amount: i128 },
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DisputeReason {
    NonDelivery,
    NotAsDescribed,
    Defective,
    WrongItem,
    LateDelivery,
    SellerNonResponsive,
    BuyerNonResponsive,
    PriceDispute,
    QualityDispute,
    Other(String),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AutomatedResolution {
    AutoReleaseToBuyer,
    AutoReleaseToSeller,
    AutoSplit { buyer_percent: u32, seller_percent: u32 },
    AutoRefund,
    RequiresArbitration,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PartyRole {
    Buyer,
    Seller,
    Arbiter,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Party {
    pub address: Address,
    pub role: PartyRole,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowTerms {
    pub amount: i128,
    pub asset: Address,
    pub release_time: u64,
    pub auto_release: bool,
    pub dispute_period: u64,
    pub arbitration_timeout: u64,
    pub partial_release_allowed: bool,
    pub min_partial_amount: i128,
    pub arbiter: Option<Address>,
    pub auto_resolution_rules: Vec<AutoResolutionRule>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutoResolutionRule {
    pub trigger: DisputeReason,
    pub resolution: AutomatedResolution,
    pub time_limit: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutomatedResolution {
    pub action: AutomatedResolutionAction,
    pub buyer_amount: Option<i128>,
    pub seller_amount: Option<i128>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AutomatedResolutionAction {
    ReleaseToBuyer,
    ReleaseToSeller,
    Split,
    Refund,
    EscalateToArbiter,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Escrow {
    pub id: u64,
    pub buyer: Address,
    pub seller: Address,
    pub arbiter: Option<Address>,
    pub terms: EscrowTerms,
    pub status: EscrowStatus,
    pub funded_amount: i128,
    pub released_amount: i128,
    pub dispute: Option<Dispute>,
    pub created_at: u64,
    pub updated_at: u64,
    pub released_at: Option<u64>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dispute {
    pub id: u64,
    pub escrow_id: u64,
    pub raised_by: Address,
    pub reason: DisputeReason,
    pub description: String,
    pub status: DisputeStatus,
    pub resolution: Option<DisputeResolution>,
    pub auto_resolution: Option<AutomatedResolution>,
    pub raised_at: u64,
    pub resolved_at: Option<u64>,
    pub resolution_deadline: u64,
    pub arbiter_assigned: Option<Address>,
    pub evidence: Map<Address, Vec<Evidence>>,
    pub auto_resolution_deadline: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Evidence {
    pub submitted_by: Address,
    pub description: String,
    pub data: Vec<u8>,
    pub submitted_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimeLock {
    pub escrow_id: u64,
    pub release_time: u64,
    pub amount: i128,
    pub released: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PartialRelease {
    pub escrow_id: u64,
    pub amount: i128,
    pub recipient: Address,
    pub reason: String,
    pub approved_by: Address,
    pub released_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyRecovery {
    pub escrow_id: u64,
    pub requested_by: Address,
    pub reason: String,
    pub approved: bool,
    pub executed_at: Option<u64>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowCreated {
    pub escrow_id: u64,
    pub buyer: Address,
    pub seller: Address,
    pub amount: i128,
    pub asset: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FundsDeposited {
    pub escrow_id: u64,
    pub amount: i128,
    pub depositor: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FundsReleased {
    pub escrow_id: u64,
    pub amount: i128,
    pub recipient: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeRaised {
    pub dispute_id: u64,
    pub escrow_id: u64,
    pub raised_by: Address,
    pub reason: DisputeReason,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeResolved {
    pub dispute_id: u64,
    pub resolution: DisputeResolution,
    pub resolved_by: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutoResolutionExecuted {
    pub dispute_id: u64,
    pub resolution: AutomatedResolution,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PartialReleaseEvent {
    pub escrow_id: u64,
    pub amount: i128,
    pub recipient: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyRecoveryEvent {
    pub escrow_id: u64,
    pub requested_by: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    AlreadyInitialized,
    NotInitialized,
    Unauthorized,
    EscrowNotFound,
    EscrowNotFunded,
    EscrowAlreadyFunded,
    InvalidAmount,
    InsufficientFunds,
    InvalidStatus,
    DisputeNotFound,
    DisputeAlreadyExists,
    DisputeNotOpen,
    DisputeExpired,
    InvalidDisputeReason,
    InvalidResolution,
    UnauthorizedResolution,
    TimeLockNotExpired,
    TimeLockExpired,
    InvalidPartialAmount,
    PartialReleaseNotAllowed,
    EmergencyRecoveryNotAllowed,
    EmergencyRecoveryAlreadyRequested,
    ArbiterNotSet,
    ArbiterAlreadyAssigned,
    InvalidArbiter,
    EvidenceNotFound,
    InvalidEvidence,
    AutoResolutionNotConfigured,
    AutoResolutionTimeout,
    ArbitrationTimeout,
    InvalidTimeLock,
    AlreadyReleased,
    AlreadyCancelled,
    Expired,
}

impl From<Error> for soroban_sdk::Error {
    fn from(e: Error) -> Self {
        soroban_sdk::Error::from_contract_error(e as u32)
    }
}

pub const ESCROW_COUNTER: Symbol = symbol_short!("ESCROW_CNT");
pub const DISPUTE_COUNTER: Symbol = symbol_short!("DISP_CNT");
pub const ESCROW_PREFIX: Symbol = symbol_short!("ESCROW");
pub const DISPUTE_PREFIX: Symbol = symbol_short!("DISP");
pub const TIMELOCK_PREFIX: Symbol = symbol_short!("TL");
pub const PARTIAL_PREFIX: Symbol = symbol_short!("PART");
pub const EMERGENCY_PREFIX: Symbol = symbol_short!("EMERG");
pub const EVIDENCE_PREFIX: Symbol = symbol_short!("EVID");
pub const ADMIN: Symbol = symbol_short!("ADMIN");
pub const PAUSED: Symbol = symbol_short!("PAUSED");