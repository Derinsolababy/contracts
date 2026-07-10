// Time-locked Release Module
use soroban_sdk::{contract, contractimpl, contracttype, contracterror, Env, Address, Symbol, symbol_short, token, log};
use crate::shared::{
    Error,
    TimeLock,
    TIMELOCK_PREFIX,
};

#[contract]
pub struct TimeLockRelease;

#[contractimpl]
impl TimeLockRelease {
    pub fn create_time_lock(
        env: Env,
        escrow_id: u64,
        release_time: u64,
        amount: i128,
    ) -> Result<(), Error> {
        if release_time < env.ledger().timestamp() {
            return Err(Error::InvalidTimeLock);
        }

        let time_lock = TimeLock {
            escrow_id,
            release_time,
            amount,
            released: false,
        };

        let key = (TIMELOCK_PREFIX, escrow_id, release_time);
        env.storage().persistent().set(&key.0, &(key.1, key.2), &time_lock);

        Ok(())
    }

    pub fn execute_time_lock(env: Env, escrow_id: u64, release_time: u64) -> Result<(), Error> {
        let key = (TIMELOCK_PREFIX, escrow_id, release_time);
        let mut time_lock: TimeLock = env.storage().persistent().get(&key.0, &(key.1, key.2)).ok_or(Error::EscrowNotFound)?;

        if time_lock.released {
            return Err(Error::AlreadyReleased);
        }

        let current_time = env.ledger().timestamp();
        if current_time < time_lock.release_time {
            return Err(Error::TimeLockNotExpired);
        }

        time_lock.released = true;
        env.storage().persistent().set(&key.0, &(key.1, key.2), &time_lock);

        log!(&env, "Time lock executed", escrow_id, release_time, time_lock.amount);

        Ok(())
    }

    pub fn get_time_lock(env: Env, escrow_id: u64, release_time: u64) -> Result<TimeLock, Error> {
        let key = (TIMELOCK_PREFIX, escrow_id, release_time);
        env.storage().persistent().get(&key.0, &(key.1, key.2)).ok_or(Error::EscrowNotFound)
    }

    pub fn is_time_lock_expired(env: Env, escrow_id: u64, release_time: u64) -> Result<bool, Error> {
        let current_time = env.ledger().timestamp();
        Ok(current_time >= release_time)
    }

    pub fn get_time_until_release(env: Env, release_time: u64) -> Result<u64, Error> {
        let current_time = env.ledger().timestamp();
        if current_time >= release_time {
            Ok(0)
        } else {
            Ok(release_time - current_time)
        }
    }

    pub fn cancel_time_lock(env: Env, escrow_id: u64, release_time: u64, caller: Address) -> Result<(), Error> {
        caller.require_auth();

        let key = (TIMELOCK_PREFIX, escrow_id, release_time);
        let mut time_lock: TimeLock = env.storage().persistent().get(&key.0, &(key.1, key.2)).ok_or(Error::EscrowNotFound)?;

        if time_lock.released {
            return Err(Error::AlreadyReleased);
        }

        env.storage().persistent().remove(&key.0, &(key.1, key.2));

        log!(&env, "Time lock cancelled", escrow_id, release_time);

        Ok(())
    }
}