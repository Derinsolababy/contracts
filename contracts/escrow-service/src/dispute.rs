// Dispute Resolution Module with Automated Resolution and Time-Bound Processes
use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror,
    Address, Env, String, Vec, Map, Symbol, symbol_short,
    log, panic_with_error,
};
use crate::shared::{
    EscrowStatus, DisputeStatus, DisputeResolution, DisputeReason,
    AutomatedResolution, AutomatedResolutionAction, PartyRole,
    Party, EscrowTerms, AutoResolutionRule,
    Escrow, Dispute, Evidence, TimeLock, PartialRelease,
    EmergencyRecovery,
    EscrowCreated, FundsDeposited, FundsReleased,
    DisputeRaised, DisputeResolved, AutoResolutionExecuted,
    PartialReleaseEvent, EmergencyRecoveryEvent,
    Error,
    ESCROW_COUNTER, DISPUTE_COUNTER, ESCROW_PREFIX, DISPUTE_PREFIX,
    TIMELOCK_PREFIX, PARTIAL_PREFIX, EMERGENCY_PREFIX, EVIDENCE_PREFIX,
    ADMIN, PAUSED,
};

#[contract]
pub struct DisputeResolution;

#[contractimpl]
impl DisputeResolution {
    // =========================================================================
    // AUTOMATED DISPUTE RESOLUTION FOR COMMON SCENARIOS
    // =========================================================================

    pub fn raise_dispute(
        env: Env,
        escrow_id: u64,
        raised_by: Address,
        reason: DisputeReason,
        description: String,
    ) -> Result<u64, Error> {
        let paused: bool = env.storage().instance().get(&PAUSED).unwrap_or(false);
        if paused {
            return Err(Error::Unauthorized);
        }

        raised_by.require_auth();

        let mut escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &escrow_id).ok_or(Error::EscrowNotFound)?;

        if escrow.status != EscrowStatus::Funded && escrow.status != EscrowStatus::Active {
            return Err(Error::InvalidStatus);
        }

        if escrow.dispute.is_some() {
            return Err(Error::DisputeAlreadyExists);
        }

        if escrow.terms.amount - escrow.released_amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let mut dispute_counter: u64 = env.storage().instance().get(&DISPUTE_COUNTER).unwrap_or(0);
        dispute_counter += 1;

        let current_time = env.ledger().timestamp();
        let resolution_deadline = current_time + escrow.terms.dispute_period;
        let arbitration_deadline = resolution_deadline + escrow.terms.arbitration_timeout;

        let auto_resolution = Self::determine_auto_resolution(&env, &escrow, &reason)?;
        let auto_resolution_deadline = if auto_resolution.is_some() {
            current_time + escrow.terms.dispute_period / 2
        } else {
            0
        };

        let dispute = Dispute {
            id: dispute_counter,
            escrow_id,
            raised_by: raised_by.clone(),
            reason: reason.clone(),
            description,
            status: DisputeStatus::Open,
            resolution: None,
            auto_resolution: auto_resolution.clone(),
            raised_at: current_time,
            resolved_at: None,
            resolution_deadline,
            arbiter_assigned: escrow.terms.arbiter.clone(),
            evidence: Map::new(&env),
            auto_resolution_deadline,
        };

        escrow.status = EscrowStatus::Disputed;
        escrow.dispute = Some(dispute.clone());
        escrow.updated_at = current_time;

        env.storage().persistent().set(&DISPUTE_PREFIX, &dispute_counter, &dispute);
        env.storage().persistent().set(&ESCROW_PREFIX, &escrow_id, &escrow);
        env.storage().instance().set(&DISPUTE_COUNTER, &dispute_counter);

        env.events().publish((symbol_short!("dispute_raised"),), DisputeRaised {
            dispute_id: dispute_counter,
            escrow_id,
            raised_by,
            reason,
        });

        Ok(dispute_counter)
    }

    fn determine_auto_resolution(
        env: &Env,
        escrow: &Escrow,
        reason: &DisputeReason,
    ) -> Result<Option<AutomatedResolution>, Error> {
        for rule in &escrow.terms.auto_resolution_rules {
            if Self::matches_reason(&rule.trigger, reason) {
                return Ok(Some(Self::create_automated_resolution(&rule.resolution)?));
            }
        }

        match reason {
            DisputeReason::NonDelivery => {
                if env.ledger().timestamp() > escrow.terms.release_time + 86400 * 7 {
                    return Ok(Some(AutomatedResolution {
                        action: AutomatedResolutionAction::ReleaseToBuyer,
                        buyer_amount: Some(escrow.terms.amount - escrow.released_amount),
                        seller_amount: Some(0),
                    }));
                }
            }
            DisputeReason::SellerNonResponsive => {
                if env.ledger().timestamp() > escrow.terms.release_time + 86400 * 3 {
                    return Ok(Some(AutomatedResolution {
                        action: AutomatedResolutionAction::ReleaseToBuyer,
                        buyer_amount: Some(escrow.terms.amount - escrow.released_amount),
                        seller_amount: Some(0),
                    }));
                }
            }
            DisputeReason::BuyerNonResponsive => {
                if env.ledger().timestamp() > escrow.terms.release_time + 86400 * 3 {
                    return Ok(Some(AutomatedResolution {
                        action: AutomatedResolutionAction::ReleaseToSeller,
                        buyer_amount: Some(0),
                        seller_amount: Some(escrow.terms.amount - escrow.released_amount),
                    }));
                }
            }
            DisputeReason::LateDelivery => {
                return Ok(Some(AutomatedResolution {
                    action: AutomatedResolutionAction::Split,
                    buyer_amount: Some((escrow.terms.amount - escrow.released_amount) / 2),
                    seller_amount: Some((escrow.terms.amount - escrow.released_amount) / 2),
                }));
            }
            DisputeReason::PriceDispute => {
                return Ok(Some(AutomatedResolution {
                    action: AutomatedResolutionAction::EscalateToArbiter,
                    buyer_amount: None,
                    seller_amount: None,
                }));
            }
            DisputeReason::QualityDispute => {
                return Ok(Some(AutomatedResolution {
                    action: AutomatedResolutionAction::EscalateToArbiter,
                    buyer_amount: None,
                    seller_amount: None,
                }));
            }
            _ => {}
        }

        Ok(None)
    }

    fn matches_reason(rule_reason: &DisputeReason, actual_reason: &DisputeReason) -> bool {
        match (rule_reason, actual_reason) {
            (DisputeReason::NonDelivery, DisputeReason::NonDelivery) => true,
            (DisputeReason::NotAsDescribed, DisputeReason::NotAsDescribed) => true,
            (DisputeReason::Defective, DisputeReason::Defective) => true,
            (DisputeReason::WrongItem, DisputeReason::WrongItem) => true,
            (DisputeReason::LateDelivery, DisputeReason::LateDelivery) => true,
            (DisputeReason::SellerNonResponsive, DisputeReason::SellerNonResponsive) => true,
            (DisputeReason::BuyerNonResponsive, DisputeReason::BuyerNonResponsive) => true,
            (DisputeReason::PriceDispute, DisputeReason::PriceDispute) => true,
            (DisputeReason::QualityDispute, DisputeReason::QualityDispute) => true,
            (DisputeReason::Other(a), DisputeReason::Other(b)) => a == b,
            _ => false,
        }
    }

    fn create_automated_resolution(resolution: &AutomatedResolution) -> Result<AutomatedResolution, Error> {
        Ok(resolution.clone())
    }

    // =========================================================================
    // TIME-BOUND RESOLUTION PROCESSES
    // =========================================================================

    pub fn execute_auto_resolution(env: Env, dispute_id: u64) -> Result<(), Error> {
        let mut dispute: Dispute = env.storage().persistent().get(&DISPUTE_PREFIX, &dispute_id).ok_or(Error::DisputeNotFound)?;

        if dispute.status != DisputeStatus::Open {
            return Err(Error::DisputeNotOpen);
        }

        if dispute.auto_resolution.is_none() {
            return Err(Error::AutoResolutionNotConfigured);
        }

        let current_time = env.ledger().timestamp();
        if current_time < dispute.auto_resolution_deadline {
            return Err(Error::AutoResolutionTimeout);
        }

        let auto_resolution = dispute.auto_resolution.as_ref().unwrap().clone();
        let mut escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &dispute.escrow_id).ok_or(Error::EscrowNotFound)?;

        let remaining_amount = escrow.terms.amount - escrow.released_amount;
        let token_client = token::Client::new(&env, &escrow.terms.asset);

        match auto_resolution.action {
            AutomatedResolutionAction::ReleaseToBuyer => {
                let amount = auto_resolution.buyer_amount.unwrap_or(remaining_amount);
                token_client.transfer(&env.current_contract_address(), &escrow.buyer, &amount);
                escrow.released_amount += amount;
            }
            AutomatedResolutionAction::ReleaseToSeller => {
                let amount = auto_resolution.seller_amount.unwrap_or(remaining_amount);
                token_client.transfer(&env.current_contract_address(), &escrow.seller, &amount);
                escrow.released_amount += amount;
            }
            AutomatedResolutionAction::Split => {
                let buyer_amount = auto_resolution.buyer_amount.unwrap_or(remaining_amount / 2);
                let seller_amount = auto_resolution.seller_amount.unwrap_or(remaining_amount - buyer_amount);
                token_client.transfer(&env.current_contract_address(), &escrow.buyer, &buyer_amount);
                token_client.transfer(&env.current_contract_address(), &escrow.seller, &seller_amount);
                escrow.released_amount += buyer_amount + seller_amount;
            }
            AutomatedResolutionAction::Refund => {
                token_client.transfer(&env.current_contract_address(), &escrow.buyer, &remaining_amount);
                escrow.released_amount += remaining_amount;
            }
            AutomatedResolutionAction::EscalateToArbiter => {
                dispute.status = DisputeStatus::UnderReview;
                dispute.arbiter_assigned = escrow.terms.arbiter.clone();
                escrow.updated_at = current_time;
                env.storage().persistent().set(&DISPUTE_PREFIX, &dispute_id, &dispute);
                env.storage().persistent().set(&ESCROW_PREFIX, &dispute.escrow_id, &escrow);
                return Ok(());
            }
        }

        dispute.status = DisputeStatus::Resolved;
        dispute.resolution = Some(DisputeResolution::AutoResolved);
        dispute.resolved_at = Some(current_time);
        dispute.auto_resolution = None;

        escrow.status = EscrowStatus::Resolved;
        escrow.updated_at = current_time;
        escrow.dispute = Some(dispute.clone());

        env.storage().persistent().set(&DISPUTE_PREFIX, &dispute_id, &dispute);
        env.storage().persistent().set(&ESCROW_PREFIX, &dispute.escrow_id, &escrow);

        env.events().publish((symbol_short!("auto_resolved"),), AutoResolutionExecuted {
            dispute_id,
            resolution: auto_resolution,
        });

        Ok(())
    }

    pub fn check_and_execute_timebound_resolutions(env: Env) -> Result<Vec<u64>, Error> {
        let mut executed = Vec::new(&env);
        let current_time = env.ledger().timestamp();

        let dispute_count: u64 = env.storage().instance().get(&DISPUTE_COUNTER).unwrap_or(0);

        for i in 1..=dispute_count {
            if let Some(mut dispute) = env.storage().persistent().get(&DISPUTE_PREFIX, &i) {
                if dispute.status == DisputeStatus::Open {
                    if current_time >= dispute.resolution_deadline {
                        if dispute.auto_resolution.is_some() {
                            Self::execute_auto_resolution(env.clone(), dispute.id)?;
                            executed.push_back(dispute.id);
                        } else if dispute.arbiter_assigned.is_some() {
                            dispute.status = DisputeStatus::Expired;
                            env.storage().persistent().set(&DISPUTE_PREFIX, &dispute.id, &dispute);
                            executed.push_back(dispute.id);
                        }
                    }
                }

                if dispute.status == DisputeStatus::UnderReview {
                    let mut escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &dispute.escrow_id).ok_or(Error::EscrowNotFound)?;
                    if current_time >= dispute.raised_at + escrow.terms.dispute_period + escrow.terms.arbitration_timeout {
                        dispute.status = DisputeStatus::Expired;
                        env.storage().persistent().set(&DISPUTE_PREFIX, &dispute.id, &dispute);
                        executed.push_back(dispute.id);
                    }
                }
            }
        }

        Ok(executed)
    }

    // =========================================================================
    // MANUAL DISPUTE RESOLUTION BY PARTIES OR ARBITER
    // =========================================================================

    pub fn resolve_dispute(
        env: Env,
        dispute_id: u64,
        resolution: DisputeResolution,
        resolved_by: Address,
    ) -> Result<(), Error> {
        resolved_by.require_auth();

        let mut dispute: Dispute = env.storage().persistent().get(&DISPUTE_PREFIX, &dispute_id).ok_or(Error::DisputeNotFound)?;

        if dispute.status != DisputeStatus::Open && dispute.status != DisputeStatus::UnderReview {
            return Err(Error::DisputeNotOpen);
        }

        let mut escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &dispute.escrow_id).ok_or(Error::EscrowNotFound)?;

        let is_arbiter = dispute.arbiter_assigned.as_ref() == Some(&resolved_by);
        let is_party = resolved_by == escrow.buyer || resolved_by == escrow.seller;

        if !is_arbiter && !is_party {
            return Err(Error::UnauthorizedResolution);
        }

        let current_time = env.ledger().timestamp();
        if current_time > dispute.resolution_deadline && !is_arbiter {
            return Err(Error::DisputeExpired);
        }

        let remaining_amount = escrow.terms.amount - escrow.released_amount;
        let token_client = token::Client::new(&env, &escrow.terms.asset);

        match resolution {
            DisputeResolution::ReleaseToBuyer => {
                token_client.transfer(&env.current_contract_address(), &escrow.buyer, &remaining_amount);
                escrow.released_amount += remaining_amount;
            }
            DisputeResolution::ReleaseToSeller => {
                token_client.transfer(&env.current_contract_address(), &escrow.seller, &remaining_amount);
                escrow.released_amount += remaining_amount;
            }
            DisputeResolution::Split { buyer_amount, seller_amount } => {
                if buyer_amount + seller_amount != remaining_amount {
                    return Err(Error::InvalidResolution);
                }
                token_client.transfer(&env.current_contract_address(), &escrow.buyer, &buyer_amount);
                token_client.transfer(&env.current_contract_address(), &escrow.seller, &seller_amount);
                escrow.released_amount += remaining_amount;
            }
            DisputeResolution::Refund => {
                token_client.transfer(&env.current_contract_address(), &escrow.buyer, &remaining_amount);
                escrow.released_amount += remaining_amount;
            }
            DisputeResolution::PartialRelease { buyer_amount, seller_amount } => {
                if buyer_amount + seller_amount > remaining_amount {
                    return Err(Error::InvalidResolution);
                }
                token_client.transfer(&env.current_contract_address(), &escrow.buyer, &buyer_amount);
                token_client.transfer(&env.current_contract_address(), &escrow.seller, &seller_amount);
                escrow.released_amount += buyer_amount + seller_amount;
            }
        }

        dispute.status = DisputeStatus::Resolved;
        dispute.resolution = Some(resolution.clone());
        dispute.resolved_at = Some(current_time);

        escrow.status = if escrow.released_amount >= escrow.terms.amount {
            EscrowStatus::Released
        } else {
            EscrowStatus::Resolved
        };
        escrow.updated_at = current_time;
        escrow.dispute = Some(dispute.clone());

        env.storage().persistent().set(&DISPUTE_PREFIX, &dispute_id, &dispute);
        env.storage().persistent().set(&ESCROW_PREFIX, &dispute.escrow_id, &escrow);

        env.events().publish((symbol_short!("dispute_resolved"),), DisputeResolved {
            dispute_id,
            resolution,
            resolved_by,
        });

        Ok(())
    }

    // =========================================================================
    // EVIDENCE MANAGEMENT
    // =========================================================================

    pub fn submit_evidence(
        env: Env,
        dispute_id: u64,
        submitted_by: Address,
        description: String,
        data: Vec<u8>,
    ) -> Result<(), Error> {
        submitted_by.require_auth();

        let mut dispute: Dispute = env.storage().persistent().get(&DISPUTE_PREFIX, &dispute_id).ok_or(Error::DisputeNotFound)?;

        if dispute.status != DisputeStatus::Open && dispute.status != DisputeStatus::UnderReview {
            return Err(Error::DisputeNotOpen);
        }

        if dispute.raised_by != submitted_by && dispute.arbiter_assigned.as_ref() != Some(&submitted_by) {
            let escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &dispute.escrow_id).ok_or(Error::EscrowNotFound)?;
            if escrow.buyer != submitted_by && escrow.seller != submitted_by {
                return Err(Error::Unauthorized);
            }
        }

        let evidence = Evidence {
            submitted_by: submitted_by.clone(),
            description,
            data,
            submitted_at: env.ledger().timestamp(),
        };

        let mut party_evidence = dispute.evidence.get(&submitted_by).unwrap_or(Vec::new(&env));
        party_evidence.push_back(evidence);
        dispute.evidence.set(submitted_by, party_evidence);

        env.storage().persistent().set(&DISPUTE_PREFIX, &dispute_id, &dispute);

        Ok(())
    }

    // =========================================================================
    // ESCALATION TO ARBITER
    // =========================================================================

    pub fn escalate_to_arbiter(env: Env, dispute_id: u64, escalated_by: Address) -> Result<(), Error> {
        escalated_by.require_auth();

        let mut dispute: Dispute = env.storage().persistent().get(&DISPUTE_PREFIX, &dispute_id).ok_or(Error::DisputeNotFound)?;

        if dispute.status != DisputeStatus::Open {
            return Err(Error::DisputeNotOpen);
        }

        let escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &dispute.escrow_id).ok_or(Error::EscrowNotFound)?;

        if escrow.terms.arbiter.is_none() {
            return Err(Error::ArbiterNotSet);
        }

        if escrow.buyer != escalated_by && escrow.seller != escalated_by {
            return Err(Error::Unauthorized);
        }

        dispute.status = DisputeStatus::UnderReview;
        dispute.arbiter_assigned = escrow.terms.arbiter.clone();

        env.storage().persistent().set(&DISPUTE_PREFIX, &dispute_id, &dispute);

        Ok(())
    }

    // =========================================================================
    // QUERY FUNCTIONS
    // =========================================================================

    pub fn get_dispute_status(env: Env, dispute_id: u64) -> Result<DisputeStatus, Error> {
        let dispute: Dispute = env.storage().persistent().get(&DISPUTE_PREFIX, &dispute_id).ok_or(Error::DisputeNotFound)?;
        Ok(dispute.status)
    }

    pub fn get_time_until_resolution_deadline(env: Env, dispute_id: u64) -> Result<u64, Error> {
        let dispute: Dispute = env.storage().persistent().get(&DISPUTE_PREFIX, &dispute_id).ok_or(Error::DisputeNotFound)?;
        let current_time = env.ledger().timestamp();
        if current_time >= dispute.resolution_deadline {
            Ok(0)
        } else {
            Ok(dispute.resolution_deadline - current_time)
        }
    }

    pub fn get_time_until_auto_resolution(env: Env, dispute_id: u64) -> Result<u64, Error> {
        let dispute: Dispute = env.storage().persistent().get(&DISPUTE_PREFIX, &dispute_id).ok_or(Error::DisputeNotFound)?;
        let current_time = env.ledger().timestamp();
        if dispute.auto_resolution.is_none() {
            return Err(Error::AutoResolutionNotConfigured);
        }
        if current_time >= dispute.auto_resolution_deadline {
            Ok(0)
        } else {
            Ok(dispute.auto_resolution_deadline - current_time)
        }
    }

    pub fn is_dispute_expired(env: Env, dispute_id: u64) -> Result<bool, Error> {
        let dispute: Dispute = env.storage().persistent().get(&DISPUTE_PREFIX, &dispute_id).ok_or(Error::DisputeNotFound)?;
        let current_time = env.ledger().timestamp();
        Ok(current_time >= dispute.resolution_deadline)
    }
}