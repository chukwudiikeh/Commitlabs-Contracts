#![no_std]
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, Env, String,
    Symbol, Vec,
};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitmentRules {
    pub duration_days: u32,
    pub max_loss_percent: u32,
    pub commitment_type: String, // "safe", "balanced", "aggressive"
    pub early_exit_penalty: u32,
    pub min_fee_threshold: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Commitment {
    pub commitment_id: String,
    pub owner: Address,
    pub nft_token_id: u32,
    pub rules: CommitmentRules,
    pub amount: i128,
    pub asset_address: Address,
    pub created_at: u64,
    pub expires_at: u64,
    pub current_value: i128,
    pub status: String, // "active", "settled", "violated", "early_exit"
}

#[contract]
pub struct CommitmentCoreContract;

// Storage keys - using Symbol for efficient storage (max 9 chars)
fn commitment_key(_e: &Env) -> Symbol {
    symbol_short!("Commit")
}

fn admin_key(_e: &Env) -> Symbol {
    symbol_short!("Admin")
}

fn nft_contract_key(_e: &Env) -> Symbol {
    symbol_short!("NFT")
}

// Error types for better error handling
#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CommitmentError {
    NotFound = 1,
    AlreadySettled = 2,
    NotExpired = 3,
    Unauthorized = 4,
    InvalidRules = 5,
    InsufficientBalance = 6,
    TransferFailed = 7,
    InvalidAmount = 8,
    AssetNotFound = 9,
}

// Storage helpers
fn read_commitment(e: &Env, commitment_id: &String) -> Option<Commitment> {
    let key = (commitment_key(e), commitment_id.clone());
    e.storage().persistent().get(&key)
}

fn set_commitment(e: &Env, commitment: &Commitment) {
    let key = (commitment_key(e), commitment.commitment_id.clone());
    e.storage().persistent().set(&key, commitment);
}

fn has_commitment(e: &Env, commitment_id: &String) -> bool {
    let key = (commitment_key(e), commitment_id.clone());
    e.storage().persistent().has(&key)
}

// ============================================================================
// Asset Transfer Helper Functions
// ============================================================================

/// Transfer tokens from user to contract
/// Verifies balance and authorization before transfer
/// Panics if transfer fails or balance insufficient
fn transfer_from_user_to_contract(e: &Env, asset: &Address, from: &Address, amount: i128) {
    assert!(amount > 0, "Amount must be positive");

    // Check user balance
    let balance = token::Client::new(e, asset).balance(from);
    assert!(balance >= amount, "Insufficient balance");

    // Get contract address
    let contract_address = e.current_contract_address();

    // Transfer tokens - this requires authorization from 'from' address
    // The transfer will panic if authorization fails or transfer is invalid
    token::Client::new(e, asset).transfer(from, &contract_address, &amount);
}

/// Transfer tokens from contract to user
/// Panics if contract has insufficient balance
fn transfer_from_contract_to_user(e: &Env, asset: &Address, to: &Address, amount: i128) {
    assert!(amount > 0, "Amount must be positive");

    // Check contract balance
    let contract_address = e.current_contract_address();
    let balance = token::Client::new(e, asset).balance(&contract_address);
    assert!(balance >= amount, "Insufficient contract balance");

    // Transfer tokens
    token::Client::new(e, asset).transfer(&contract_address, to, &amount);
}

/// Transfer tokens from contract to pool
/// Panics if contract has insufficient balance
fn transfer_from_contract_to_pool(e: &Env, asset: &Address, pool: &Address, amount: i128) {
    assert!(amount > 0, "Amount must be positive");

    // Check contract balance
    let contract_address = e.current_contract_address();
    let balance = token::Client::new(e, asset).balance(&contract_address);
    assert!(balance >= amount, "Insufficient contract balance");

    // Transfer tokens
    token::Client::new(e, asset).transfer(&contract_address, pool, &amount);
}

/// Get balance of an address for a specific asset
fn get_balance(e: &Env, asset: &Address, address: &Address) -> i128 {
    token::Client::new(e, asset).balance(address)
}

/// Verify that a user has sufficient balance for transfer
/// Returns error if balance insufficient
fn verify_sufficient_balance(
    e: &Env,
    asset: &Address,
    user: &Address,
    amount: i128,
) -> Result<(), CommitmentError> {
    let balance = get_balance(e, asset, user);
    if balance < amount {
        return Err(CommitmentError::InsufficientBalance);
    }
    Ok(())
}

#[contractimpl]
impl CommitmentCoreContract {
    /// Initialize the core commitment contract
    pub fn initialize(e: Env, admin: Address, nft_contract: Address) {
        // Store admin
        e.storage().instance().set(&admin_key(&e), &admin);
        // Store NFT contract address
        e.storage()
            .instance()
            .set(&nft_contract_key(&e), &nft_contract);
    }

    /// Create a new commitment
    pub fn create_commitment(
        e: Env,
        owner: Address,
        amount: i128,
        asset_address: Address,
        rules: CommitmentRules,
    ) -> Result<String, CommitmentError> {
        // Require authorization from owner
        owner.require_auth();

        // Validate rules
        if rules.duration_days == 0 {
            return Err(CommitmentError::InvalidRules);
        }
        if rules.max_loss_percent > 100 {
            return Err(CommitmentError::InvalidRules);
        }
        if amount <= 0 {
            return Err(CommitmentError::InvalidAmount);
        }

        // Verify user has sufficient balance
        verify_sufficient_balance(&e, &asset_address, &owner, amount)?;

        // Transfer assets from owner to contract
        transfer_from_user_to_contract(&e, &asset_address, &owner, amount);

        // Generate unique commitment ID based on timestamp
        let timestamp = e.ledger().timestamp();
        let commitment_id = String::from_str(&e, "commitment_");
        // In production, would append timestamp/hash for uniqueness

        // Calculate expiration
        let duration_seconds = (rules.duration_days as u64) * 24 * 60 * 60;
        let expires_at = timestamp + duration_seconds;

        // Get NFT contract and mint NFT
        let nft_contract: Address = e.storage().instance().get(&nft_contract_key(&e)).unwrap();
        let nft_token_id: u32 = 1; // This will be returned from NFT contract mint call
                                   // TODO: Call NFT contract to mint (requires cross-contract call implementation)

        // Create commitment
        let commitment = Commitment {
            commitment_id: commitment_id.clone(),
            owner: owner.clone(),
            nft_token_id,
            rules,
            amount,
            asset_address: asset_address.clone(),
            created_at: timestamp,
            expires_at,
            current_value: amount, // Initially same as amount
            status: String::from_str(&e, "active"),
        };

        // Store commitment
        set_commitment(&e, &commitment);

        // Emit creation event
        e.events().publish(
            (symbol_short!("create"), commitment_id.clone()),
            (owner, amount, asset_address),
        );

        Ok(commitment_id)
    }

    /// Get commitment details
    pub fn get_commitment(e: Env, commitment_id: String) -> Commitment {
        read_commitment(&e, &commitment_id).unwrap_or_else(|| panic!("Commitment not found"))
    }

    /// Update commitment value (called by allocation logic)
    pub fn update_value(_e: Env, _commitment_id: String, _new_value: i128) {
        // TODO: Verify caller is authorized (allocation contract)
        // TODO: Update current_value
        // TODO: Check if max_loss_percent is violated
        // TODO: Emit value update event
    }

    /// Check if commitment rules are violated
    /// Returns true if any rule violation is detected (loss limit or duration)
    pub fn check_violations(e: Env, commitment_id: String) -> bool {
        let commitment =
            read_commitment(&e, &commitment_id).unwrap_or_else(|| panic!("Commitment not found"));

        // Skip check if already settled or violated
        let active_status = String::from_str(&e, "active");
        if commitment.status != active_status {
            return false; // Already processed
        }

        let current_time = e.ledger().timestamp();

        // Check loss limit violation
        // Calculate loss percentage: ((amount - current_value) / amount) * 100
        let loss_amount = commitment.amount - commitment.current_value;
        let loss_percent = if commitment.amount > 0 {
            // Use i128 arithmetic to avoid overflow
            // loss_percent = (loss_amount * 100) / amount
            (loss_amount * 100) / commitment.amount
        } else {
            0
        };

        // Convert max_loss_percent (u32) to i128 for comparison
        let max_loss = commitment.rules.max_loss_percent as i128;
        let loss_violated = loss_percent > max_loss;

        // Check duration violation (expired)
        let duration_violated = current_time >= commitment.expires_at;

        // Return true if any violation exists
        loss_violated || duration_violated
    }

    /// Get detailed violation information
    /// Returns a tuple: (has_violations, loss_violated, duration_violated, loss_percent, time_remaining)
    pub fn get_violation_details(e: Env, commitment_id: String) -> (bool, bool, bool, i128, u64) {
        let commitment =
            read_commitment(&e, &commitment_id).unwrap_or_else(|| panic!("Commitment not found"));

        let current_time = e.ledger().timestamp();

        // Calculate loss percentage
        let loss_amount = commitment.amount - commitment.current_value;
        let loss_percent = if commitment.amount > 0 {
            (loss_amount * 100) / commitment.amount
        } else {
            0
        };

        // Check loss limit violation
        let max_loss = commitment.rules.max_loss_percent as i128;
        let loss_violated = loss_percent > max_loss;

        // Check duration violation
        let duration_violated = current_time >= commitment.expires_at;

        // Calculate time remaining (0 if expired)
        let time_remaining = if current_time < commitment.expires_at {
            commitment.expires_at - current_time
        } else {
            0
        };

        let has_violations = loss_violated || duration_violated;

        (
            has_violations,
            loss_violated,
            duration_violated,
            loss_percent,
            time_remaining,
        )
    }

    /// Settle commitment at maturity
    pub fn settle(e: Env, commitment_id: String) -> Result<(), CommitmentError> {
        // Get commitment
        let mut commitment =
            read_commitment(&e, &commitment_id).ok_or(CommitmentError::NotFound)?;

        // Verify commitment is expired
        let current_time = e.ledger().timestamp();
        if current_time < commitment.expires_at {
            return Err(CommitmentError::NotExpired);
        }

        // Check if already settled
        let active_status = String::from_str(&e, "active");
        if commitment.status != active_status {
            return Err(CommitmentError::AlreadySettled);
        }

        // Calculate final settlement amount (use current_value)
        let settlement_amount = commitment.current_value;

        // Transfer assets back to owner
        if settlement_amount > 0 {
            transfer_from_contract_to_user(
                &e,
                &commitment.asset_address,
                &commitment.owner,
                settlement_amount,
            );
        }

        // Mark commitment as settled
        commitment.status = String::from_str(&e, "settled");
        set_commitment(&e, &commitment);

        // Call NFT contract to mark NFT as settled
        // TODO: Implement cross-contract call to NFT settle function

        // Emit settlement event
        e.events().publish(
            (symbol_short!("settle"), commitment_id),
            (commitment.owner, settlement_amount),
        );

        Ok(())
    }

    /// Early exit (with penalty)
    pub fn early_exit(
        e: Env,
        commitment_id: String,
        caller: Address,
    ) -> Result<(), CommitmentError> {
        // Require authorization from caller
        caller.require_auth();

        // Get commitment
        let mut commitment =
            read_commitment(&e, &commitment_id).ok_or(CommitmentError::NotFound)?;

        // Verify caller is owner
        if caller != commitment.owner {
            return Err(CommitmentError::Unauthorized);
        }

        // Check if already settled
        let active_status = String::from_str(&e, "active");
        if commitment.status != active_status {
            return Err(CommitmentError::AlreadySettled);
        }

        // Calculate penalty
        // Penalty = current_value * (early_exit_penalty / 100)
        let penalty_percent = commitment.rules.early_exit_penalty as i128;
        let penalty_amount = (commitment.current_value * penalty_percent) / 100;
        let remaining_amount = commitment.current_value - penalty_amount;

        // Transfer remaining amount (after penalty) to owner
        if remaining_amount > 0 {
            transfer_from_contract_to_user(
                &e,
                &commitment.asset_address,
                &commitment.owner,
                remaining_amount,
            );
        }

        // Mark commitment as early_exit
        commitment.status = String::from_str(&e, "early_exit");
        set_commitment(&e, &commitment);

        // Emit early exit event
        e.events().publish(
            (symbol_short!("earlyexit"), commitment_id),
            (caller, remaining_amount, penalty_amount),
        );

        Ok(())
    }

    /// Allocate liquidity (called by allocation strategy)
    pub fn allocate(
        e: Env,
        commitment_id: String,
        target_pool: Address,
        amount: i128,
    ) -> Result<(), CommitmentError> {
        // Get commitment
        let commitment = read_commitment(&e, &commitment_id).ok_or(CommitmentError::NotFound)?;

        // Verify commitment is active
        let active_status = String::from_str(&e, "active");
        if commitment.status != active_status {
            return Err(CommitmentError::AlreadySettled);
        }

        // TODO: Verify caller is authorized allocation contract
        // This would require storing authorized allocators in contract state

        // Transfer assets to target pool
        transfer_from_contract_to_pool(&e, &commitment.asset_address, &target_pool, amount);

        // TODO: Record allocation in storage for tracking

        // Emit allocation event
        e.events().publish(
            (symbol_short!("allocate"), commitment_id),
            (target_pool, amount),
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests;
