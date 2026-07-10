#[cfg(test)]
mod tests {
    use soroban_sdk::{
        testutils::{Address as _, AuthorizedFunction, AuthorizedInvocation, Ledger, LedgerInfo},
        Address, Env, String, Vec, Map, symbol_short, IntoVal,
    };
    use crate::shared::{
        EscrowStatus, DisputeStatus, DisputeResolution, DisputeReason,
        AutomatedResolution, AutomatedResolutionAction, PartyRole,
        Party, EscrowTerms, AutoResolutionRule,
        Escrow, Dispute, Evidence, TimeLock, PartialRelease,
        EmergencyRecovery,
        Error,
    };
    use crate::{EscrowService, DisputeResolution as DisputeModule, TimeLockRelease};

    fn create_test_env() -> Env {
        let env = Env::default();
        env.mock_all_auths();
        env
    }

    fn setup_contract(env: &Env) -> (Address, Address, Address, Address) {
        let admin = Address::generate(&env);
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let arbiter = Address::generate(&env);

        EscrowService::initialize(env.clone(), admin.clone());

        (admin, buyer, seller, arbiter)
    }

    fn create_mock_token(env: &Env, admin: &Address) -> Address {
        let token_admin = Address::generate(env);
        let token = soroban_token_sdk::TokenClient::new(env, &token_admin);
        token_admin
    }

    #[test]
    fn test_create_escrow() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() + 86400;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            None,
            Vec::new(&env),
        ).unwrap();

        assert_eq!(escrow_id, 1);

        let escrow = EscrowService::get_escrow(env.clone(), escrow_id).unwrap();
        assert_eq!(escrow.id, 1);
        assert_eq!(escrow.buyer, buyer);
        assert_eq!(escrow.seller, seller);
        assert_eq!(escrow.terms.amount, amount);
        assert_eq!(escrow.status, EscrowStatus::Created);
    }

    #[test]
    fn test_deposit_and_fund_escrow() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() + 86400;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            None,
            Vec::new(&env),
        ).unwrap();

        let deposit_amount: i128 = 500;
        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), deposit_amount).unwrap();

        let escrow = EscrowService::get_escrow(env.clone(), escrow_id).unwrap();
        assert_eq!(escrow.funded_amount, deposit_amount);
        assert_eq!(escrow.status, EscrowStatus::Created);
    }

    #[test]
    fn test_full_funding_transitions_to_funded() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() + 86400;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            None,
            Vec::new(&env),
        ).unwrap();

        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), amount).unwrap();

        let escrow = EscrowService::get_escrow(env.clone(), escrow_id).unwrap();
        assert_eq!(escrow.funded_amount, amount);
        assert_eq!(escrow.status, EscrowStatus::Funded);
    }

    #[test]
    fn test_raise_dispute() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() + 86400;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            None,
            Vec::new(&env),
        ).unwrap();

        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), amount).unwrap();

        let reason = DisputeReason::NonDelivery;
        let dispute_id = DisputeModule::raise_dispute(
            env.clone(),
            escrow_id,
            buyer.clone(),
            reason.clone(),
            String::from_str(&env, "Seller did not deliver"),
        ).unwrap();

        assert_eq!(dispute_id, 1);

        let dispute = DisputeModule::get_dispute(env.clone(), dispute_id).unwrap();
        assert_eq!(dispute.escrow_id, escrow_id);
        assert_eq!(dispute.raised_by, buyer);
        assert_eq!(dispute.reason, reason);
        assert_eq!(dispute.status, DisputeStatus::Open);

        let escrow = EscrowService::get_escrow(env.clone(), escrow_id).unwrap();
        assert_eq!(escrow.status, EscrowStatus::Disputed);
    }

    #[test]
    fn test_automated_resolution_non_delivery() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() - 86400 * 8;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let auto_rules = Vec::from_array(&env, [
            AutoResolutionRule {
                trigger: DisputeReason::NonDelivery,
                resolution: AutomatedResolution {
                    action: AutomatedResolutionAction::ReleaseToBuyer,
                    buyer_amount: Some(1000),
                    seller_amount: Some(0),
                },
                time_limit: 86400 * 10,
            }
        ]);

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            None,
            auto_rules,
        ).unwrap();

        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), amount).unwrap();

        let dispute_id = DisputeModule::raise_dispute(
            env.clone(),
            escrow_id,
            buyer.clone(),
            DisputeReason::NonDelivery,
            String::from_str(&env, "Not delivered"),
        ).unwrap();

        let escrow = EscrowService::get_escrow(env.clone(), escrow_id).unwrap();
        let dispute = escrow.dispute.unwrap();

        assert!(dispute.auto_resolution.is_some());
        let auto_res = dispute.auto_resolution.unwrap();
        assert_eq!(auto_res.action, AutomatedResolutionAction::ReleaseToBuyer);
    }

    #[test]
    fn test_automated_resolution_seller_non_responsive() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() - 86400 * 5;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            None,
            Vec::new(&env),
        ).unwrap();

        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), amount).unwrap();

        let dispute_id = DisputeModule::raise_dispute(
            env.clone(),
            escrow_id,
            buyer.clone(),
            DisputeReason::SellerNonResponsive,
            String::from_str(&env, "Seller not responding"),
        ).unwrap();

        let escrow = EscrowService::get_escrow(env.clone(), escrow_id).unwrap();
        let dispute = escrow.dispute.unwrap();

        assert!(dispute.auto_resolution.is_some());
        let auto_res = dispute.auto_resolution.unwrap();
        assert_eq!(auto_res.action, AutomatedResolutionAction::ReleaseToBuyer);
    }

    #[test]
    fn test_automated_resolution_buyer_non_responsive() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() - 86400 * 5;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            None,
            Vec::new(&env),
        ).unwrap();

        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), amount).unwrap();

        let dispute_id = DisputeModule::raise_dispute(
            env.clone(),
            escrow_id,
            seller.clone(),
            DisputeReason::BuyerNonResponsive,
            String::from_str(&env, "Buyer not responding"),
        ).unwrap();

        let escrow = EscrowService::get_escrow(env.clone(), escrow_id).unwrap();
        let dispute = escrow.dispute.unwrap();

        assert!(dispute.auto_resolution.is_some());
        let auto_res = dispute.auto_resolution.unwrap();
        assert_eq!(auto_res.action, AutomatedResolutionAction::ReleaseToSeller);
    }

    #[test]
    fn test_automated_resolution_late_delivery_split() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() + 86400;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            None,
            Vec::new(&env),
        ).unwrap();

        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), amount).unwrap();

        let dispute_id = DisputeModule::raise_dispute(
            env.clone(),
            escrow_id,
            buyer.clone(),
            DisputeReason::LateDelivery,
            String::from_str(&env, "Late delivery"),
        ).unwrap();

        let escrow = EscrowService::get_escrow(env.clone(), escrow_id).unwrap();
        let dispute = escrow.dispute.unwrap();

        assert!(dispute.auto_resolution.is_some());
        let auto_res = dispute.auto_resolution.unwrap();
        assert_eq!(auto_res.action, AutomatedResolutionAction::Split);
    }

    #[test]
    fn test_automated_resolution_price_dispute_escalates() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() + 86400;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            None,
            Vec::new(&env),
        ).unwrap();

        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), amount).unwrap();

        let dispute_id = DisputeModule::raise_dispute(
            env.clone(),
            escrow_id,
            buyer.clone(),
            DisputeReason::PriceDispute,
            String::from_str(&env, "Price dispute"),
        ).unwrap();

        let escrow = EscrowService::get_escrow(env.clone(), escrow_id).unwrap();
        let dispute = escrow.dispute.unwrap();

        assert!(dispute.auto_resolution.is_some());
        let auto_res = dispute.auto_resolution.unwrap();
        assert_eq!(auto_res.action, AutomatedResolutionAction::EscalateToArbiter);
    }

    #[test]
    fn test_manual_dispute_resolution_by_arbiter() {
        let env = create_test_env();
        let (admin, buyer, seller, arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() + 86400;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            Some(arbiter.clone()),
            Vec::new(&env),
        ).unwrap();

        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), amount).unwrap();

        let dispute_id = DisputeModule::raise_dispute(
            env.clone(),
            escrow_id,
            buyer.clone(),
            DisputeReason::NonDelivery,
            String::from_str(&env, "Not delivered"),
        ).unwrap();

        let resolution = DisputeResolution::ReleaseToBuyer;
        DisputeModule::resolve_dispute(
            env.clone(),
            dispute_id,
            arbiter.clone(),
            resolution.clone(),
        ).unwrap();

        let dispute = DisputeModule::get_dispute(env.clone(), dispute_id).unwrap();
        assert_eq!(dispute.status, DisputeStatus::Resolved);
        assert_eq!(dispute.resolution, Some(resolution));
    }

    #[test]
    fn test_manual_dispute_resolution_split() {
        let env = create_test_env();
        let (admin, buyer, seller, arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() + 86400;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            Some(arbiter.clone()),
            Vec::new(&env),
        ).unwrap();

        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), amount).unwrap();

        let dispute_id = DisputeModule::raise_dispute(
            env.clone(),
            escrow_id,
            buyer.clone(),
            DisputeReason::NotAsDescribed,
            String::from_str(&env, "Not as described"),
        ).unwrap();

        let resolution = DisputeResolution::Split { buyer_amount: 600, seller_amount: 400 };
        DisputeModule::resolve_dispute(
            env.clone(),
            dispute_id,
            arbiter.clone(),
            resolution.clone(),
        ).unwrap();

        let dispute = DisputeModule::get_dispute(env.clone(), dispute_id).unwrap();
        assert_eq!(dispute.status, DisputeStatus::Resolved);
        assert_eq!(dispute.resolution, Some(resolution));
    }

    #[test]
    fn test_time_until_resolution_deadline() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() + 86400;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            None,
            Vec::new(&env),
        ).unwrap();

        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), amount).unwrap();

        let dispute_id = DisputeModule::raise_dispute(
            env.clone(),
            escrow_id,
            buyer.clone(),
            DisputeReason::NonDelivery,
            String::from_str(&env, "Not delivered"),
        ).unwrap();

        let time_left = DisputeModule::get_time_until_resolution_deadline(env.clone(), dispute_id).unwrap();
        assert!(time_left > 0);
        assert!(time_left <= dispute_period);
    }

    #[test]
    fn test_time_until_auto_resolution() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() - 86400 * 8;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let auto_rules = Vec::from_array(&env, [
            AutoResolutionRule {
                trigger: DisputeReason::NonDelivery,
                resolution: AutomatedResolution {
                    action: AutomatedResolutionAction::ReleaseToBuyer,
                    buyer_amount: Some(1000),
                    seller_amount: Some(0),
                },
                time_limit: 86400 * 10,
            }
        ]);

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            None,
            auto_rules,
        ).unwrap();

        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), amount).unwrap();

        let dispute_id = DisputeModule::raise_dispute(
            env.clone(),
            escrow_id,
            buyer.clone(),
            DisputeReason::NonDelivery,
            String::from_str(&env, "Not delivered"),
        ).unwrap();

        let time_left = DisputeModule::get_time_until_auto_resolution(env.clone(), dispute_id).unwrap();
        assert!(time_left > 0);
    }

    #[test]
    fn test_is_dispute_expired() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() + 86400;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            None,
            Vec::new(&env),
        ).unwrap();

        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), amount).unwrap();

        let dispute_id = DisputeModule::raise_dispute(
            env.clone(),
            escrow_id,
            buyer.clone(),
            DisputeReason::NonDelivery,
            String::from_str(&env, "Not delivered"),
        ).unwrap();

        let expired = DisputeModule::is_dispute_expired(env.clone(), dispute_id).unwrap();
        assert!(!expired);
    }

    #[test]
    fn test_evidence_submission() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() + 86400;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            None,
            Vec::new(&env),
        ).unwrap();

        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), amount).unwrap();

        let dispute_id = DisputeModule::raise_dispute(
            env.clone(),
            escrow_id,
            buyer.clone(),
            DisputeReason::NonDelivery,
            String::from_str(&env, "Not delivered"),
        ).unwrap();

        DisputeModule::submit_evidence(
            env.clone(),
            dispute_id,
            buyer.clone(),
            String::from_str(&env, "Screenshot of order"),
            Vec::from_array(&env, [1, 2, 3, 4]),
        ).unwrap();

        let evidence = DisputeModule::get_evidence(env.clone(), dispute_id, buyer.clone()).unwrap();
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence.get(0).unwrap().submitted_by, buyer);
    }

    #[test]
    fn test_escalate_to_arbiter() {
        let env = create_test_env();
        let (admin, buyer, seller, arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() + 86400;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            Some(arbiter.clone()),
            Vec::new(&env),
        ).unwrap();

        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), amount).unwrap();

        let dispute_id = DisputeModule::raise_dispute(
            env.clone(),
            escrow_id,
            buyer.clone(),
            DisputeReason::NonDelivery,
            String::from_str(&env, "Not delivered"),
        ).unwrap();

        DisputeModule::escalate_to_arbiter(env.clone(), dispute_id, buyer.clone()).unwrap();

        let dispute = DisputeModule::get_dispute(env.clone(), dispute_id).unwrap();
        assert_eq!(dispute.status, DisputeStatus::UnderReview);
        assert_eq!(dispute.arbiter_assigned, Some(arbiter));
    }

    #[test]
    fn test_partial_release() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() + 86400;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            true,
            100,
            None,
            Vec::new(&env),
        ).unwrap();

        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), amount).unwrap();

        EscrowService::partial_release(
            env.clone(),
            escrow_id,
            buyer.clone(),
            300,
            seller.clone(),
            String::from_str(&env, "Milestone 1"),
        ).unwrap();

        let escrow = EscrowService::get_escrow(env.clone(), escrow_id).unwrap();
        assert_eq!(escrow.released_amount, 300);
    }

    #[test]
    fn test_emergency_recovery() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() + 86400;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            None,
            Vec::new(&env),
        ).unwrap();

        EscrowService::deposit(env.clone(), escrow_id, buyer.clone(), amount).unwrap();

        EscrowService::request_emergency_recovery(
            env.clone(),
            escrow_id,
            admin.clone(),
            String::from_str(&env, "Emergency"),
        ).unwrap();

        let escrow = EscrowService::get_escrow(env.clone(), escrow_id).unwrap();
        assert_eq!(escrow.status, EscrowStatus::Funded);
    }

    #[test]
    fn test_cancel_escrow() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() + 86400;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            None,
            Vec::new(&env),
        ).unwrap();

        EscrowService::cancel_escrow(env.clone(), escrow_id, buyer.clone()).unwrap();

        let escrow = EscrowService::get_escrow(env.clone(), escrow_id).unwrap();
        assert_eq!(escrow.status, EscrowStatus::Cancelled);
    }

    #[test]
    fn test_time_lock_creation_and_execution() {
        let env = create_test_env();
        let (admin, buyer, seller, _arbiter) = setup_contract(&env);

        let asset = create_mock_token(&env, &admin);
        let amount: i128 = 1000;
        let release_time = env.ledger().timestamp() + 86400;
        let dispute_period = 86400 * 7;
        let arbitration_timeout = 86400 * 3;

        let escrow_id = EscrowService::create_escrow(
            env.clone(),
            buyer.clone(),
            seller.clone(),
            amount,
            asset.clone(),
            release_time,
            true,
            dispute_period,
            arbitration_timeout,
            false,
            100,
            None,
            Vec::new(&env),
        ).unwrap();

        let timelock_release_time = env.ledger().timestamp() + 86400 * 2;
        TimeLockRelease::create_time_lock(env.clone(), escrow_id, timelock_release_time, 500).unwrap();

        let time_lock = TimeLockRelease::get_time_lock(env.clone(), escrow_id, timelock_release_time).unwrap();
        assert_eq!(time_lock.escrow_id, escrow_id);
        assert_eq!(time_lock.amount, 500);
        assert!(!time_lock.released);

        let expired = TimeLockRelease::is_time_lock_expired(env.clone(), escrow_id, timelock_release_time).unwrap();
        assert!(!expired);

        env.ledger().set_timestamp(timelock_release_time + 1);

        let expired = TimeLockRelease::is_time_lock_expired(env.clone(), escrow_id, timelock_release_time).unwrap();
        assert!(expired);
    }

    fn create_mock_token(env: &Env, admin: &Address) -> Address {
        let token = soroban_token_sdk::TokenClient::new(env, admin);
        token
    }
}

fn main() {}