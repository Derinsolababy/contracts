// Escrow core logic
use soroban_sdk::{contract, contractimpl, contracttype, contracterror, Env, Address, String, Vec, Map, Symbol, symbol_short, token, log, panic_with_error};
use shared::{
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
pub struct EscrowService;

#[contractimpl]
impl EscrowService {
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&ADMIN) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&ADMIN, &admin);
        env.storage().instance().set(&PAUSED, &false);
        env.storage().instance().set(&ESCROW_COUNTER, &0u64);
        env.storage().instance().set(&DISPUTE_COUNTER, &0u64);
        Ok(())
    }

    pub fn pause(env: Env) -> Result<(), Error> {
        let admin: Address = env.storage().instance().get(&ADMIN).ok_or(Error::NotInitialized)?;
        admin.require_auth();
        env.storage().instance().set(&PAUSED, &true);
        Ok(())
    }

    pub fn unpause(env: Env) -> Result<(), Error> {
        let admin: Address = env.storage().instance().get(&ADMIN).ok_or(Error::NotInitialized)?;
        admin.require_auth();
        env.storage().instance().set(&PAUSED, &false);
        Ok(())
    }

    pub fn create_escrow(
        env: Env,
        buyer: Address,
        seller: Address,
        amount: i128,
        asset: Address,
        release_time: u64,
        auto_release: bool,
        dispute_period: u64,
        arbitration_timeout: u64,
        partial_release_allowed: bool,
        min_partial_amount: i128,
        arbiter: Option<Address>,
        auto_resolution_rules: Vec<AutoResolutionRule>,
    ) -> Result<u64, Error> {
        let paused: bool = env.storage().instance().get(&PAUSED).unwrap_or(false);
        if paused {
            return Err(Error::Unauthorized);
        }

        buyer.require_auth();

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        if min_partial_amount < 0 {
            return Err(Error::InvalidAmount);
        }
        if release_time < env.ledger().timestamp() {
            return Err(Error::InvalidTimeLock);
        }
        if dispute_period == 0 {
            return Err(Error::InvalidAmount);
        }
        if arbitration_timeout == 0 {
            return Err(Error::InvalidAmount);
        }

        let mut escrow_counter: u64 = env.storage().instance().get(&ESCROW_COUNTER).unwrap_or(0);
        escrow_counter += 1;

        let terms = EscrowTerms {
            amount,
            asset,
            release_time,
            auto_release,
            dispute_period,
            arbitration_timeout,
            partial_release_allowed,
            min_partial_amount,
            arbiter: arbiter.clone(),
            auto_resolution_rules,
        };

        let escrow = Escrow {
            id: escrow_counter,
            buyer: buyer.clone(),
            seller: seller.clone(),
            arbiter,
            terms,
            status: EscrowStatus::Created,
            funded_amount: 0,
            released_amount: 0,
            dispute: None,
            created_at: env.ledger().timestamp(),
            updated_at: env.ledger().timestamp(),
            released_at: None,
        };

        env.storage().persistent().set(&ESCROW_PREFIX, &escrow_counter, &escrow);
        env.storage().instance().set(&ESCROW_COUNTER, &escrow_counter);

        env.events().publish((symbol_short!("escrow_created"),), EscrowCreated {
            escrow_id: escrow_counter,
            buyer,
            seller,
            amount,
            asset,
        });

        Ok(escrow_counter)
    }

    pub fn deposit(env: Env, escrow_id: u64, from: Address, amount: i128) -> Result<(), Error> {
        let paused: bool = env.storage().instance().get(&PAUSED).unwrap_or(false);
        if paused {
            return Err(Error::Unauthorized);
        }

        from.require_auth();

        let mut escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &escrow_id).ok_or(Error::EscrowNotFound)?;

        if escrow.status != EscrowStatus::Created {
            return Err(Error::InvalidStatus);
        }

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        if escrow.funded_amount + amount > escrow.terms.amount {
            return Err(Error::InvalidAmount);
        }

        let token_client = token::Client::new(&env, &escrow.terms.asset);
        token_client.transfer(&from, &env.current_contract_address(), &amount);

        escrow.funded_amount += amount;
        escrow.status = if escrow.funded_amount >= escrow.terms.amount {
            EscrowStatus::Funded
        } else {
            EscrowStatus::Created
        };
        escrow.updated_at = env.ledger().timestamp();

        env.storage().persistent().set(&ESCROW_PREFIX, &escrow_id, &escrow);

        env.events().publish((symbol_short!("funds_deposited"),), FundsDeposited {
            escrow_id,
            amount,
            depositor: from,
        });

        Ok(())
    }

    pub fn release(env: Env, escrow_id: u64, by: Address) -> Result<(), Error> {
        let paused: bool = env.storage().instance().get(&PAUSED).unwrap_or(false);
        if paused {
            return Err(Error::Unauthorized);
        }

        by.require_auth();

        let mut escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &escrow_id).ok_or(Error::EscrowNotFound)?;

        if escrow.status != EscrowStatus::Funded && escrow.status != EscrowStatus::Active {
            return Err(Error::InvalidStatus);
        }

        if escrow.dispute.is_some() {
            return Err(Error::InvalidStatus);
        }

        let current_time = env.ledger().timestamp();
        if current_time < escrow.terms.release_time {
            return Err(Error::TimeLockNotExpired);
        }

        if by != escrow.seller && by != escrow.buyer && by != escrow.terms.arbiter.unwrap_or(Address::from_str(&env, "")) {
            return Err(Error::Unauthorized);
        }

        let amount_to_release = escrow.terms.amount - escrow.released_amount;
        if amount_to_release <= 0 {
            return Err(Error::AlreadyReleased);
        }

        let token_client = token::Client::new(&env, &escrow.terms.asset);
        token_client.transfer(&env.current_contract_address(), &escrow.seller, &amount_to_release);

        escrow.released_amount += amount_to_release;
        escrow.status = EscrowStatus::Released;
        escrow.released_at = Some(current_time);
        escrow.updated_at = current_time;

        env.storage().persistent().set(&ESCROW_PREFIX, &escrow_id, &escrow);

        env.events().publish((symbol_short!("funds_released"),), FundsReleased {
            escrow_id,
            amount: amount_to_release,
            recipient: escrow.seller.clone(),
        });

        Ok(())
    }

    pub fn auto_release(env: Env, escrow_id: u64) -> Result<(), Error> {
        let paused: bool = env.storage().instance().get(&PAUSED).unwrap_or(false);
        if paused {
            return Err(Error::Unauthorized);
        }

        let mut escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &escrow_id).ok_or(Error::EscrowNotFound)?;

        if escrow.status != EscrowStatus::Funded && escrow.status != EscrowStatus::Active {
            return Err(Error::InvalidStatus);
        }

        if escrow.dispute.is_some() {
            return Err(Error::InvalidStatus);
        }

        let current_time = env.ledger().timestamp();
        if current_time < escrow.terms.release_time {
            return Err(Error::TimeLockNotExpired);
        }

        if !escrow.terms.auto_release {
            return Err(Error::Unauthorized);
        }

        let amount_to_release = escrow.terms.amount - escrow.released_amount;
        if amount_to_release <= 0 {
            return Err(Error::AlreadyReleased);
        }

        let token_client = token::Client::new(&env, &escrow.terms.asset);
        token_client.transfer(&env.current_contract_address(), &escrow.seller, &amount_to_release);

        escrow.released_amount += amount_to_release;
        escrow.status = EscrowStatus::Released;
        escrow.released_at = Some(current_time);
        escrow.updated_at = current_time;

        env.storage().persistent().set(&ESCROW_PREFIX, &escrow_id, &escrow);

        env.events().publish((symbol_short!("funds_released"),), FundsReleased {
            escrow_id,
            amount: amount_to_release,
            recipient: escrow.seller.clone(),
        });

        Ok(())
    }

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

        if escrow.status == EscrowStatus::Released || escrow.status == EscrowStatus::Cancelled || escrow.status == EscrowStatus::Expired {
            return Err(Error::InvalidStatus);
        }

        if escrow.dispute.is_some() {
            return Err(Error::DisputeAlreadyExists);
        }

        if raised_by != escrow.buyer && raised_by != escrow.seller {
            return Err(Error::Unauthorized);
        }

        let current_time = env.ledger().timestamp();
        if current_time > escrow.terms.release_time + escrow.terms.dispute_period {
            return Err(Error::DisputeExpired);
        }

        let mut dispute_counter: u64 = env.storage().instance().get(&DISPUTE_COUNTER).unwrap_or(0);
        dispute_counter += 1;

        let resolution_deadline = current_time + escrow.terms.dispute_period;
        let auto_resolution_deadline = current_time + escrow.terms.arbitration_timeout;

        let auto_resolution = Self::check_automated_resolution(&env, &escrow, &reason, current_time);

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

        escrow.dispute = Some(dispute.clone());
        escrow.status = EscrowStatus::Disputed;
        escrow.updated_at = current_time;

        env.storage().persistent().set(&ESCROW_PREFIX, &escrow_id, &escrow);
        env.storage().persistent().set(&DISPUTE_PREFIX, &dispute_counter, &dispute);
        env.storage().instance().set(&DISPUTE_COUNTER, &dispute_counter);

        env.events().publish((symbol_short!("dispute_raised"),), DisputeRaised {
            dispute_id: dispute_counter,
            escrow_id,
            raised_by,
            reason,
        });

        if let Some(auto_res) = auto_resolution {
            Self::execute_automated_resolution(&env, &mut escrow, dispute_counter, auto_res)?;
        }

        Ok(dispute_counter)
    }

    fn check_automated_resolution(
        env: &Env,
        escrow: &Escrow,
        reason: &DisputeReason,
        current_time: u64,
    ) -> Option<AutomatedResolution> {
        for rule in &escrow.terms.auto_resolution_rules {
            if rule.trigger == *reason {
                if current_time <= rule.time_limit {
                    return Some(AutomatedResolution {
                        action: rule.resolution.action.clone(),
                        buyer_amount: rule.resolution.buyer_amount,
                        seller_amount: rule.resolution.seller_amount,
                    });
                }
            }
        }
        None
    }

    fn execute_automated_resolution(
        env: &Env,
        escrow: &mut Escrow,
        dispute_id: u64,
        auto_res: AutomatedResolution,
    ) -> Result<(), Error> {
        let current_time = env.ledger().timestamp();
        let dispute = escrow.dispute.as_mut().ok_or(Error::DisputeNotFound)?;

        let remaining_amount = escrow.terms.amount - escrow.released_amount;
        let token_client = token::Client::new(env, &escrow.terms.asset);

        match auto_res.action {
            AutomatedResolutionAction::ReleaseToBuyer => {
                token_client.transfer(&env.current_contract_address(), &escrow.buyer, &remaining_amount);
                escrow.released_amount += remaining_amount;
            }
            AutomatedResolutionAction::ReleaseToSeller => {
                token_client.transfer(&env.current_contract_address(), &escrow.seller, &remaining_amount);
                escrow.released_amount += remaining_amount;
            }
            AutomatedResolutionAction::Split => {
                let buyer_amt = auto_res.buyer_amount.unwrap_or(remaining_amount / 2);
                let seller_amt = auto_res.seller_amount.unwrap_or(remaining_amount - buyer_amt);
                token_client.transfer(&env.current_contract_address(), &escrow.buyer, &buyer_amt);
                token_client.transfer(&env.current_contract_address(), &escrow.seller, &seller_amt);
                escrow.released_amount += buyer_amt + seller_amt;
            }
            AutomatedResolutionAction::Refund => {
                token_client.transfer(&env.current_contract_address(), &escrow.buyer, &remaining_amount);
                escrow.released_amount += remaining_amount;
            }
            AutomatedResolutionAction::EscalateToArbiter => {
                dispute.status = DisputeStatus::UnderReview;
                dispute.auto_resolution = Some(auto_res);
                env.storage().persistent().set(&DISPUTE_PREFIX, &dispute_id, dispute);
                env.storage().persistent().set(&ESCROW_PREFIX, &escrow.id, escrow);
                return Ok(());
            }
        }

        let resolution = DisputeResolution::AutoResolved;

        dispute.resolution = Some(resolution.clone());
        dispute.status = DisputeStatus::Resolved;
        dispute.resolved_at = Some(current_time);
        dispute.auto_resolution = Some(auto_res.clone());

        escrow.status = EscrowStatus::Resolved;
        escrow.updated_at = current_time;

        env.storage().persistent().set(&DISPUTE_PREFIX, &dispute_id, dispute);
        env.storage().persistent().set(&ESCROW_PREFIX, &escrow.id, escrow);

        env.events().publish((symbol_short!("auto_resolution"),), AutoResolutionExecuted {
            dispute_id,
            resolution: auto_res,
        });

        env.events().publish((symbol_short!("dispute_resolved"),), DisputeResolved {
            dispute_id,
            resolution,
            resolved_by: env.current_contract_address(),
        });

        Ok(())
    }

    pub fn submit_evidence(
        env: Env,
        dispute_id: u64,
        submitted_by: Address,
        description: String,
        data: Vec<u8>,
    ) -> Result<(), Error> {
        let paused: bool = env.storage().instance().get(&PAUSED).unwrap_or(false);
        if paused {
            return Err(Error::Unauthorized);
        }

        submitted_by.require_auth();

        let mut dispute: Dispute = env.storage().persistent().get(&DISPUTE_PREFIX, &dispute_id).ok_or(Error::DisputeNotFound)?;

        if dispute.status != DisputeStatus::Open && dispute.status != DisputeStatus::UnderReview {
            return Err(Error::DisputeNotOpen);
        }

        if submitted_by != dispute.raised_by && submitted_by != dispute.arbiter_assigned.unwrap_or(Address::from_str(&env, "")) {
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

        let mut evidences = dispute.evidence.get(&submitted_by).unwrap_or(Vec::new(&env));
        evidences.push_back(evidence);
        dispute.evidence.set(submitted_by, evidences);

        env.storage().persistent().set(&DISPUTE_PREFIX, &dispute_id, &dispute);

        env.events().publish((symbol_short!("evidence_submitted"),), (dispute_id, submitted_by));

        Ok(())
    }

    pub fn resolve_dispute(
        env: Env,
        dispute_id: u64,
        resolver: Address,
        resolution: DisputeResolution,
    ) -> Result<(), Error> {
        let paused: bool = env.storage().instance().get(&PAUSED).unwrap_or(false);
        if paused {
            return Err(Error::Unauthorized);
        }

        resolver.require_auth();

        let mut dispute: Dispute = env.storage().persistent().get(&DISPUTE_PREFIX, &dispute_id).ok_or(Error::DisputeNotFound)?;

        if dispute.status != DisputeStatus::Open && dispute.status != DisputeStatus::UnderReview {
            return Err(Error::DisputeNotOpen);
        }

        let escrow_id = dispute.escrow_id;
        let mut escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &escrow_id).ok_or(Error::EscrowNotFound)?;

        let is_arbiter = dispute.arbiter_assigned.as_ref() == Some(&resolver);
        let is_party = resolver == escrow.buyer || resolver == escrow.seller;

        if !is_arbiter && !is_party {
            return Err(Error::UnauthorizedResolution);
        }

        let current_time = env.ledger().timestamp();
        if current_time > dispute.resolution_deadline && !is_arbiter {
            return Err(Error::ArbitrationTimeout);
        }

        let remaining_amount = escrow.terms.amount - escrow.released_amount;
        let token_client = token::Client::new(&env, &escrow.terms.asset);

        let (buyer_amt, seller_amt) = match &resolution {
            DisputeResolution::ReleaseToBuyer => (remaining_amount, 0),
            DisputeResolution::ReleaseToSeller => (0, remaining_amount),
            DisputeResolution::Split { buyer_amount, seller_amount } => (*buyer_amount, *seller_amount),
            DisputeResolution::Refund => (remaining_amount, 0),
            DisputeResolution::PartialRelease { buyer_amount, seller_amount } => (*buyer_amount, *seller_amount),
            DisputeResolution::AutoResolved => {
                return Err(Error::InvalidResolution);
            }
        };

        if buyer_amt + seller_amt > remaining_amount {
            return Err(Error::InvalidResolution);
        }

        if buyer_amt > 0 {
            token_client.transfer(&env.current_contract_address(), &escrow.buyer, &buyer_amt);
            escrow.released_amount += buyer_amt;
        }

        if seller_amt > 0 {
            token_client.transfer(&env.current_contract_address(), &escrow.seller, &seller_amt);
            escrow.released_amount += seller_amt;
        }

        dispute.resolution = Some(resolution.clone());
        dispute.status = DisputeStatus::Resolved;
        dispute.resolved_at = Some(current_time);

        escrow.status = if escrow.released_amount >= escrow.terms.amount {
            EscrowStatus::Released
        } else {
            EscrowStatus::Resolved
        };
        escrow.updated_at = current_time;

        env.storage().persistent().set(&DISPUTE_PREFIX, &dispute_id, &dispute);
        env.storage().persistent().set(&ESCROW_PREFIX, &escrow_id, &escrow);

        env.events().publish((symbol_short!("dispute_resolved"),), DisputeResolved {
            dispute_id,
            resolution,
            resolved_by: resolver,
        });

        Ok(())
    }

    pub fn execute_auto_resolution(env: Env, dispute_id: u64) -> Result<(), Error> {
        let paused: bool = env.storage().instance().get(&PAUSED).unwrap_or(false);
        if paused {
            return Err(Error::Unauthorized);
        }

        let mut dispute: Dispute = env.storage().persistent().get(&DISPUTE_PREFIX, &dispute_id).ok_or(Error::DisputeNotFound)?;

        if dispute.status != DisputeStatus::Open && dispute.status != DisputeStatus::UnderReview {
            return Err(Error::DisputeNotOpen);
        }

        let current_time = env.ledger().timestamp();
        if current_time < dispute.auto_resolution_deadline {
            return Err(Error::AutoResolutionTimeout);
        }

        let auto_res = dispute.auto_resolution.clone().ok_or(Error::AutoResolutionNotConfigured)?;
        let escrow_id = dispute.escrow_id;
        let mut escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &escrow_id).ok_or(Error::EscrowNotFound)?;

        Self::execute_automated_resolution(&env, &mut escrow, dispute_id, auto_res)?;

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

    pub fn partial_release(
        env: Env,
        escrow_id: u64,
        requester: Address,
        amount: i128,
        recipient: Address,
        reason: String,
    ) -> Result<(), Error> {
        let paused: bool = env.storage().instance().get(&PAUSED).unwrap_or(false);
        if paused {
            return Err(Error::Unauthorized);
        }

        requester.require_auth();

        let mut escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &escrow_id).ok_or(Error::EscrowNotFound)?;

        if !escrow.terms.partial_release_allowed {
            return Err(Error::PartialReleaseNotAllowed);
        }

        if amount < escrow.terms.min_partial_amount {
            return Err(Error::InvalidPartialAmount);
        }

        if amount > escrow.terms.amount - escrow.released_amount {
            return Err(Error::InvalidPartialAmount);
        }

        if recipient != escrow.buyer && recipient != escrow.seller {
            return Err(Error::Unauthorized);
        }

        if escrow.dispute.is_some() {
            let dispute = escrow.dispute.as_ref().unwrap();
            if dispute.status == DisputeStatus::Open || dispute.status == DisputeStatus::UnderReview {
                return Err(Error::InvalidStatus);
            }
        }

        let token_client = token::Client::new(&env, &escrow.terms.asset);
        token_client.transfer(&env.current_contract_address(), &recipient, &amount);

        escrow.released_amount += amount;
        escrow.updated_at = env.ledger().timestamp();

        let partial_release = PartialRelease {
            escrow_id,
            amount,
            recipient: recipient.clone(),
            reason,
            approved_by: requester,
            released_at: env.ledger().timestamp(),
        };

        let key = (PARTIAL_PREFIX, escrow_id, escrow.released_amount);
        env.storage().persistent().set(&key.0, &(key.1, key.2), &partial_release);

        env.storage().persistent().set(&ESCROW_PREFIX, &escrow_id, &escrow);

        env.events().publish((symbol_short!("partial_release"),), PartialReleaseEvent {
            escrow_id,
            amount,
            recipient,
        });

        Ok(())
    }

    pub fn request_emergency_recovery(
        env: Env,
        escrow_id: u64,
        requested_by: Address,
        reason: String,
    ) -> Result<(), Error> {
        let paused: bool = env.storage().instance().get(&PAUSED).unwrap_or(false);
        if paused {
            return Err(Error::Unauthorized);
        }

        requested_by.require_auth();

        let admin: Address = env.storage().instance().get(&ADMIN).ok_or(Error::NotInitialized)?;

        if requested_by != admin {
            return Err(Error::Unauthorized);
        }

        let mut escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &escrow_id).ok_or(Error::EscrowNotFound)?;

        if escrow.status == EscrowStatus::Released || escrow.status == EscrowStatus::Cancelled {
            return Err(Error::EmergencyRecoveryNotAllowed);
        }

        let key = (EMERGENCY_PREFIX, escrow_id);
        if env.storage().persistent().has(&key.0, &key.1) {
            return Err(Error::EmergencyRecoveryAlreadyRequested);
        }

        let recovery = EmergencyRecovery {
            escrow_id,
            requested_by: requested_by.clone(),
            reason,
            approved: false,
            executed_at: None,
        };

        env.storage().persistent().set(&key.0, &key.1, &recovery);

        env.events().publish((symbol_short!("emergency_recovery"),), EmergencyRecoveryEvent {
            escrow_id,
            requested_by,
            amount: escrow.terms.amount - escrow.released_amount,
        });

        Ok(())
    }

    pub fn execute_emergency_recovery(env: Env, escrow_id: u64) -> Result<(), Error> {
        let admin: Address = env.storage().instance().get(&ADMIN).ok_or(Error::NotInitialized)?;
        admin.require_auth();

        let mut escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &escrow_id).ok_or(Error::EscrowNotFound)?;

        let key = (EMERGENCY_PREFIX, escrow_id);
        let mut recovery: EmergencyRecovery = env.storage().persistent().get(&key.0, &key.1).ok_or(Error::EscrowNotFound)?;

        if recovery.approved {
            return Err(Error::EmergencyRecoveryNotAllowed);
        }

        let remaining_amount = escrow.terms.amount - escrow.released_amount;

        let token_client = token::Client::new(&env, &escrow.terms.asset);
        token_client.transfer(&env.current_contract_address(), &admin, &remaining_amount);

        escrow.released_amount += remaining_amount;
        escrow.status = EscrowStatus::Released;
        escrow.updated_at = env.ledger().timestamp();

        recovery.approved = true;
        recovery.executed_at = Some(env.ledger().timestamp());

        env.storage().persistent().set(&ESCROW_PREFIX, &escrow_id, &escrow);
        env.storage().persistent().set(&key.0, &key.1, &recovery);

        Ok(())
    }

    pub fn cancel_escrow(env: Env, escrow_id: u64, by: Address) -> Result<(), Error> {
        let paused: bool = env.storage().instance().get(&PAUSED).unwrap_or(false);
        if paused {
            return Err(Error::Unauthorized);
        }

        by.require_auth();

        let mut escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &escrow_id).ok_or(Error::EscrowNotFound)?;

        if escrow.status == EscrowStatus::Released || escrow.status == EscrowStatus::Cancelled {
            return Err(Error::AlreadyCancelled);
        }

        if by != escrow.buyer && by != escrow.seller && by != escrow.terms.arbiter.unwrap_or(Address::from_str(&env, "")) {
            return Err(Error::Unauthorized);
        }

        if escrow.funded_amount > 0 {
            let token_client = token::Client::new(&env, &escrow.terms.asset);
            token_client.transfer(&env.current_contract_address(), &escrow.buyer, &escrow.funded_amount);
        }

        escrow.status = EscrowStatus::Cancelled;
        escrow.updated_at = env.ledger().timestamp();

        env.storage().persistent().set(&ESCROW_PREFIX, &escrow_id, &escrow);

        Ok(())
    }

    pub fn get_escrow(env: Env, escrow_id: u64) -> Result<Escrow, Error> {
        env.storage().persistent().get(&ESCROW_PREFIX, &escrow_id).ok_or(Error::EscrowNotFound)
    }

    pub fn get_dispute(env: Env, dispute_id: u64) -> Result<Dispute, Error> {
        env.storage().persistent().get(&DISPUTE_PREFIX, &dispute_id).ok_or(Error::DisputeNotFound)
    }

    pub fn get_escrow_status(env: Env, escrow_id: u64) -> Result<EscrowStatus, Error> {
        let escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &escrow_id).ok_or(Error::EscrowNotFound)?;
        Ok(escrow.status)
    }

    pub fn get_dispute_status(env: Env, dispute_id: u64) -> Result<DisputeStatus, Error> {
        let dispute: Dispute = env.storage().persistent().get(&DISPUTE_PREFIX, &dispute_id).ok_or(Error::DisputeNotFound)?;
        Ok(dispute.status)
    }

    pub fn get_time_until_release(env: Env, escrow_id: u64) -> Result<u64, Error> {
        let escrow: Escrow = env.storage().persistent().get(&ESCROW_PREFIX, &escrow_id).ok_or(Error::EscrowNotFound)?;
        let current_time = env.ledger().timestamp();
        if current_time >= escrow.terms.release_time {
            Ok(0)
        } else {
            Ok(escrow.terms.release_time - current_time)
        }
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
}