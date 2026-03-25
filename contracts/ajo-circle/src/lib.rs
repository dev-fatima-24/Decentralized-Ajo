#![no_std]

pub mod factory;

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, token, Address, BytesN, Env, Map, Vec};

const MAX_MEMBERS: u32 = 50;
const HARD_CAP: u32 = 100;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AjoError {
    NotFound = 1,
    Unauthorized = 2,
    AlreadyExists = 3,
    InvalidInput = 4,
    AlreadyPaid = 5,
    InsufficientFunds = 6,
    Disqualified = 7,
    VoteAlreadyActive = 8,
    NoActiveVote = 9,
    AlreadyVoted = 10,
    CircleNotActive = 11,
    CircleAlreadyDissolved = 12,
    CircleAtCapacity = 13,
    CirclePanicked = 14,
    PriceUnavailable = 15,
    ArithmeticOverflow = 16,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CircleData {
    pub organizer: Address,
    pub token_address: Address,
    pub contribution_amount: i128,
    pub frequency_days: u32,
    pub max_rounds: u32,
    pub current_round: u32,
    pub member_count: u32,
    pub max_members: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberData {
    pub address: Address,
    pub total_contributed: i128,
    pub total_withdrawn: i128,
    pub has_received_payout: bool,
    pub status: u32, // 0 = Active, 1 = Inactive, 2 = Exited
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CircleStatus {
    Active,
    VotingForDissolution,
    Dissolved,
    Panicked,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DissolutionVote {
    pub votes_for: u32,
    pub total_members: u32,
    pub threshold_mode: u32, // 0 = simple majority, 1 = supermajority
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberStanding {
    pub missed_count: u32,
    pub is_active: bool,
}

#[contracttype]
pub enum DataKey {
    Circle,
    Members,
    Standings,
    Admin,
    KycStatus,
    CircleStatus,
    DissolutionVote,
    VoteCast,
    RotationOrder,
    RoundDeadline,
    RoundContribCount,
    EthUsdPrice,
    EthUsdDecimals,
    LastDepositAt,
    TotalPool,
}

#[contract]
pub struct AjoCircle;

#[contractimpl]
impl AjoCircle {
    fn require_admin(env: &Env, admin: &Address) -> Result<(), AjoError> {
        admin.require_auth();
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).ok_or(AjoError::NotFound)?;
        if stored_admin != *admin {
            return Err(AjoError::Unauthorized);
        }
        Ok(())
    }

    fn pow10_checked(exp: u32) -> Result<i128, AjoError> {
        let mut result: i128 = 1;
        for _ in 0..exp {
            result = result.checked_mul(10).ok_or(AjoError::ArithmeticOverflow)?;
        }
        Ok(result)
    }

    pub fn initialize_circle(
        env: Env,
        organizer: Address,
        token_address: Address,
        contribution_amount: i128,
        frequency_days: u32,
        max_rounds: u32,
        max_members: u32,
    ) -> Result<(), AjoError> {
        organizer.require_auth();
        let configured_max_members = if max_members == 0 { MAX_MEMBERS } else { max_members };

        if contribution_amount <= 0 || frequency_days == 0 || max_rounds == 0 || configured_max_members == 0 || configured_max_members > HARD_CAP {
            return Err(AjoError::InvalidInput);
        }

        let circle_data = CircleData {
            organizer: organizer.clone(),
            token_address,
            contribution_amount,
            frequency_days,
            max_rounds,
            current_round: 1,
            member_count: 1,
            max_members: configured_max_members,
        };

        env.storage().instance().set(&DataKey::Circle, &circle_data);
        env.storage().instance().set(&DataKey::Admin, &organizer);
        env.storage().instance().set(&DataKey::RoundContribCount, &0_u32);

        let deadline = env.ledger().timestamp() + (frequency_days as u64) * 86_400;
        env.storage().instance().set(&DataKey::RoundDeadline, &deadline);

        let mut members: Map<Address, MemberData> = Map::new(&env);
        members.set(organizer.clone(), MemberData {
            address: organizer.clone(),
            total_contributed: 0,
            total_withdrawn: 0,
            has_received_payout: false,
            status: 0,
        });
        env.storage().instance().set(&DataKey::Members, &members);

        let mut standings: Map<Address, MemberStanding> = Map::new(&env);
        standings.set(organizer.clone(), MemberStanding { missed_count: 0, is_active: true });
        env.storage().instance().set(&DataKey::Standings, &standings);

        Ok(())
    }

    pub fn join_circle(env: Env, organizer: Address, new_member: Address) -> Result<(), AjoError> {
        organizer.require_auth();
        if Self::get_circle_status(env.clone()) == CircleStatus::Panicked {
            return Err(AjoError::CirclePanicked);
        }

        let mut circle: CircleData = env.storage().instance().get(&DataKey::Circle).ok_or(AjoError::NotFound)?;
        if circle.organizer != organizer {
            return Err(AjoError::Unauthorized);
        }

        let mut members: Map<Address, MemberData> = env.storage().instance().get(&DataKey::Members).ok_or(AjoError::NotFound)?;
        if members.contains_key(new_member.clone()) {
            return Err(AjoError::AlreadyExists);
        }

        if circle.member_count >= circle.max_members {
            return Err(AjoError::CircleAtCapacity);
        }

        members.set(new_member.clone(), MemberData {
            address: new_member.clone(),
            total_contributed: 0,
            total_withdrawn: 0,
            has_received_payout: false,
            status: 0,
        });

        circle.member_count = circle.member_count.checked_add(1).ok_or(AjoError::InvalidInput)?;

        let mut standings: Map<Address, MemberStanding> = env.storage().instance().get(&DataKey::Standings).unwrap_or(Map::new(&env));
        standings.set(new_member.clone(), MemberStanding { missed_count: 0, is_active: true });

        env.storage().instance().set(&DataKey::Members, &members);
        env.storage().instance().set(&DataKey::Circle, &circle);
        env.storage().instance().set(&DataKey::Standings, &standings);

        Ok(())
    }

    pub fn add_member(env: Env, organizer: Address, new_member: Address) -> Result<(), AjoError> {
        Self::join_circle(env, organizer, new_member)
    }

    pub fn contribute(env: Env, member: Address, amount: i128) -> Result<(), AjoError> {
        member.require_auth();
        if Self::get_circle_status(env.clone()) == CircleStatus::Panicked {
            return Err(AjoError::CirclePanicked);
        }
        if amount <= 0 {
            return Err(AjoError::InvalidInput);
        }

        let mut circle: CircleData = env.storage().instance().get(&DataKey::Circle).ok_or(AjoError::NotFound)?;
        let mut standings: Map<Address, MemberStanding> = env.storage().instance().get(&DataKey::Standings).ok_or(AjoError::NotFound)?;

        let mut standing = standings.get(member.clone()).ok_or(AjoError::NotFound)?;
        if standing.missed_count >= 3 {
             panic!("Member disqualified due to inactivity.");
        }
        if !standing.is_active {
            return Err(AjoError::Disqualified);
        }
        standing.missed_count = 0;
        standings.set(member.clone(), standing);
        env.storage().instance().set(&DataKey::Standings, &standings);

        let mut members: Map<Address, MemberData> = env.storage().instance().get(&DataKey::Members).ok_or(AjoError::NotFound)?;
        let mut member_data = members.get(member.clone()).ok_or(AjoError::NotFound)?;

        let round_target = (circle.current_round as i128).checked_mul(circle.contribution_amount).ok_or(AjoError::ArithmeticOverflow)?;
        let had_completed_round = member_data.total_contributed >= round_target;

        let token_client = token::Client::new(&env, &circle.token_address);
        token_client.transfer(&member, &env.current_contract_address(), &amount);

        member_data.total_contributed = member_data.total_contributed.checked_add(amount).ok_or(AjoError::ArithmeticOverflow)?;
        let has_completed_round = member_data.total_contributed >= round_target;

        members.set(member.clone(), member_data);
        env.storage().instance().set(&DataKey::Members, &members);

        if !had_completed_round && has_completed_round {
            let mut round_contrib_count: u32 = env.storage().instance().get(&DataKey::RoundContribCount).unwrap_or(0);
            round_contrib_count = round_contrib_count.checked_add(1).ok_or(AjoError::ArithmeticOverflow)?;

            if round_contrib_count >= circle.member_count {
                let deadline: u64 = env.storage().instance().get(&DataKey::RoundDeadline).unwrap_or(0);
                let next_deadline = deadline + (circle.frequency_days as u64) * 86_400;
                env.storage().instance().set(&DataKey::RoundDeadline, &next_deadline);

                if circle.current_round < circle.max_rounds {
                    circle.current_round += 1;
                    env.storage().instance().set(&DataKey::Circle, &circle);
                }
                round_contrib_count = 0;
            }
            env.storage().instance().set(&DataKey::RoundContribCount, &round_contrib_count);
        }
        Ok(())
    }

    pub fn deposit(env: Env, member: Address) -> Result<(), AjoError> {
        member.require_auth();
        let mut circle: CircleData = env.storage().instance().get(&DataKey::Circle).ok_or(AjoError::NotFound)?;
        let amount = circle.contribution_amount;

        Self::contribute(env.clone(), member.clone(), amount)?;

        // Update accounting
        let ts = env.ledger().timestamp();
        let mut last_deposits: Map<Address, u64> = env.storage().instance().get(&DataKey::LastDepositAt).unwrap_or_else(|| Map::new(&env));
        last_deposits.set(member.clone(), ts);
        env.storage().instance().set(&DataKey::LastDepositAt, &last_deposits);

        let mut pool: i128 = env.storage().instance().get(&DataKey::TotalPool).unwrap_or(0);
        pool = pool.checked_add(amount).ok_or(AjoError::ArithmeticOverflow)?;
        env.storage().instance().set(&DataKey::TotalPool, &pool);

        Ok(())
    }

    pub fn claim_payout(env: Env, member: Address) -> Result<i128, AjoError> {
        member.require_auth();
        if Self::get_circle_status(env.clone()) == CircleStatus::Panicked {
            return Err(AjoError::CirclePanicked);
        }

        let circle: CircleData = env.storage().instance().get(&DataKey::Circle).ok_or(AjoError::NotFound)?;
        let standings: Map<Address, MemberStanding> = env.storage().instance().get(&DataKey::Standings).unwrap_or(Map::new(&env));

        if let Some(standing) = standings.get(member.clone()) {
            if !standing.is_active {
                return Err(AjoError::Disqualified);
            }
        }

        let mut members: Map<Address, MemberData> = env.storage().instance().get(&DataKey::Members).ok_or(AjoError::NotFound)?;

        if let Some(rotation) = env.storage().instance().get::<DataKey, Vec<Address>>(&DataKey::RotationOrder) {
            let idx = (circle.current_round - 1) as u32;
            let expected = rotation.get(idx).ok_or(AjoError::InvalidInput)?;
            if expected != member {
                return Err(AjoError::Unauthorized);
            }
        }

        let mut member_data = members.get(member.clone()).ok_or(AjoError::NotFound)?;
        if member_data.has_received_payout {
            return Err(AjoError::AlreadyPaid);
        }

        let payout = (circle.member_count as i128) * circle.contribution_amount;

        let token_client = token::Client::new(&env, &circle.token_address);
        token_client.transfer(&env.current_contract_address(), &member, &payout);

        member_data.has_received_payout = true;
        member_data.total_withdrawn += payout;

        members.set(member, member_data);
        env.storage().instance().set(&DataKey::Members, &members);

        Ok(payout)
    }

    pub fn partial_withdraw(env: Env, member: Address, amount: i128) -> Result<i128, AjoError> {
        member.require_auth();
        if Self::get_circle_status(env.clone()) == CircleStatus::Panicked {
            return Err(AjoError::CirclePanicked);
        }
        if amount <= 0 {
            return Err(AjoError::InvalidInput);
        }

        let mut members: Map<Address, MemberData> = env.storage().instance().get(&DataKey::Members).ok_or(AjoError::NotFound)?;
        let mut member_data = members.get(member.clone()).ok_or(AjoError::NotFound)?;

        let available = member_data.total_contributed - member_data.total_withdrawn;
        if amount > available {
            return Err(AjoError::InsufficientFunds);
        }

        let net_amount = amount - (amount * 10) / 100;
        let circle: CircleData = env.storage().instance().get(&DataKey::Circle).ok_or(AjoError::NotFound)?;

        let token_client = token::Client::new(&env, &circle.token_address);
        token_client.transfer(&env.current_contract_address(), &member, &net_amount);

        member_data.total_withdrawn += amount;
        members.set(member, member_data);
        env.storage().instance().set(&DataKey::Members, &members);

        Ok(net_amount)
    }

    pub fn get_circle_state(env: Env) -> Result<CircleData, AjoError> {
        env.storage().instance().get(&DataKey::Circle).ok_or(AjoError::NotFound)
    }

    pub fn get_member_balance(env: Env, member: Address) -> Result<MemberData, AjoError> {
        let members: Map<Address, MemberData> = env.storage().instance().get(&DataKey::Members).ok_or(AjoError::NotFound)?;
        members.get(member).ok_or(AjoError::NotFound)
    }

    pub fn get_members(env: Env) -> Result<Vec<MemberData>, AjoError> {
        let members: Map<Address, MemberData> = env.storage().instance().get(&DataKey::Members).ok_or(AjoError::NotFound)?;
        let mut members_vec = Vec::new(&env);
        for (_, member) in members.iter() {
            members_vec.push_back(member);
        }
        Ok(members_vec)
    }

    pub fn start_dissolution_vote(env: Env, caller: Address, threshold_mode: u32) -> Result<(), AjoError> {
        caller.require_auth();
        if threshold_mode > 1 { return Err(AjoError::InvalidInput); }

        let circle: CircleData = env.storage().instance().get(&DataKey::Circle).ok_or(AjoError::NotFound)?;
        let status = Self::get_circle_status(env.clone());

        match status {
            CircleStatus::Dissolved => return Err(AjoError::CircleAlreadyDissolved),
            CircleStatus::VotingForDissolution => return Err(AjoError::VoteAlreadyActive),
            CircleStatus::Panicked => return Err(AjoError::CirclePanicked),
            CircleStatus::Active => {}
        }

        let members: Map<Address, MemberData> = env.storage().instance().get(&DataKey::Members).ok_or(AjoError::NotFound)?;
        if !members.contains_key(caller.clone()) && circle.organizer != caller {
            return Err(AjoError::Unauthorized);
        }

        let vote = DissolutionVote {
            votes_for: 0,
            total_members: circle.member_count,
            threshold_mode,
        };

        env.storage().instance().set(&DataKey::CircleStatus, &CircleStatus::VotingForDissolution);
        env.storage().instance().set(&DataKey::DissolutionVote, &vote);
        env.storage().instance().set(&DataKey::VoteCast, &Map::<Address, bool>::new(&env));

        Ok(())
    }

    pub fn vote_to_dissolve(env: Env, member: Address) -> Result<(), AjoError> {
        member.require_auth();
        if Self::get_circle_status(env.clone()) != CircleStatus::VotingForDissolution {
            return Err(AjoError::NoActiveVote);
        }

        let members: Map<Address, MemberData> = env.storage().instance().get(&DataKey::Members).ok_or(AjoError::NotFound)?;
        if !members.contains_key(member.clone()) {
            return Err(AjoError::Unauthorized);
        }

        let mut vote_cast: Map<Address, bool> = env.storage().instance().get(&DataKey::VoteCast).unwrap_or_else(|| Map::new(&env));
        if vote_cast.get(member.clone()).unwrap_or(false) {
            return Err(AjoError::AlreadyVoted);
        }

        vote_cast.set(member.clone(), true);
        env.storage().instance().set(&DataKey::VoteCast, &vote_cast);

        let mut vote: DissolutionVote = env.storage().instance().get(&DataKey::DissolutionVote).ok_or(AjoError::NoActiveVote)?;
        vote.votes_for += 1;

        let threshold_met = if vote.threshold_mode == 1 {
            vote.votes_for * 100 > vote.total_members * 66
        } else {
            vote.votes_for * 2 > vote.total_members
        };

        if threshold_met {
            env.storage().instance().set(&DataKey::CircleStatus, &CircleStatus::Dissolved);
        }

        env.storage().instance().set(&DataKey::DissolutionVote, &vote);
        Ok(())
    }

    pub fn dissolve_and_refund(env: Env, member: Address) -> Result<i128, AjoError> {
        member.require_auth();
        if Self::get_circle_status(env.clone()) != CircleStatus::Dissolved {
            return Err(AjoError::CircleNotActive);
        }

        let mut members: Map<Address, MemberData> = env.storage().instance().get(&DataKey::Members).ok_or(AjoError::NotFound)?;
        let mut member_data = members.get(member.clone()).ok_or(AjoError::NotFound)?;

        let refund = member_data.total_contributed - member_data.total_withdrawn;
        if refund <= 0 {
            return Err(AjoError::InsufficientFunds);
        }

        let circle: CircleData = env.storage().instance().get(&DataKey::Circle).ok_or(AjoError::NotFound)?;
        let token_client = token::Client::new(&env, &circle.token_address);
        token_client.transfer(&env.current_contract_address(), &member, &refund);

        member_data.total_withdrawn += refund;
        member_data.status = 2; // Exited
        members.set(member, member_data);
        env.storage().instance().set(&DataKey::Members, &members);
        Ok(refund)
    }

    pub fn get_circle_status(env: Env) -> CircleStatus {
        env.storage().instance().get(&DataKey::CircleStatus).unwrap_or(CircleStatus::Active)
    }

    pub fn get_dissolution_vote(env: Env) -> Result<DissolutionVote, AjoError> {
        env.storage().instance().get(&DataKey::DissolutionVote).ok_or(AjoError::NoActiveVote)
    }

    pub fn panic(env: Env, admin: Address) -> Result<(), AjoError> {
        Self::require_admin(&env, &admin)?;
        let status = Self::get_circle_status(env.clone());
        if status == CircleStatus::Dissolved { return Err(AjoError::CircleAlreadyDissolved); }
        if status == CircleStatus::Panicked { return Err(AjoError::CirclePanicked); }

        env.storage().instance().set(&DataKey::CircleStatus, &CircleStatus::Panicked);
        Ok(())
    }

    pub fn emergency_refund(env: Env, member: Address) -> Result<i128, AjoError> {
        member.require_auth();
        if Self::get_circle_status(env.clone()) != CircleStatus::Panicked {
            return Err(AjoError::CircleNotActive);
        }

        let mut members: Map<Address, MemberData> = env.storage().instance().get(&DataKey::Members).ok_or(AjoError::NotFound)?;
        let mut member_data = members.get(member.clone()).ok_or(AjoError::NotFound)?;

        let refund = member_data.total_contributed - member_data.total_withdrawn;
        if refund <= 0 { return Err(AjoError::InsufficientFunds); }

        let circle: CircleData = env.storage().instance().get(&DataKey::Circle).ok_or(AjoError::NotFound)?;
        let token_client = token::Client::new(&env, &circle.token_address);
        token_client.transfer(&env.current_contract_address(), &member, &refund);

        member_data.total_withdrawn += refund;
        member_data.status = 2; // Exited
        members.set(member, member_data);
        env.storage().instance().set(&DataKey::Members, &members);
        Ok(refund)
    }

    pub fn is_panicked(env: Env) -> bool {
        Self::get_circle_status(env) == CircleStatus::Panicked
    }

    pub fn is_kyc_verified(env: Env, member: Address) -> bool {
        let kyc: Map<Address, bool> = env.storage().instance().get(&DataKey::KycStatus).unwrap_or_else(|| Map::new(&env));
        kyc.get(member).unwrap_or(false)
    }

    pub fn set_kyc_status(env: Env, admin: Address, member: Address, is_verified: bool) -> Result<(), AjoError> {
        Self::require_admin(&env, &admin)?;
        let mut kyc: Map<Address, bool> = env.storage().instance().get(&DataKey::KycStatus).unwrap_or_else(|| Map::new(&env));
        kyc.set(member, is_verified);
        env.storage().instance().set(&DataKey::KycStatus, &kyc);
        Ok(())
    }

    pub fn set_eth_usd_price(env: Env, admin: Address, price: i128, decimals: u32) -> Result<(), AjoError> {
        Self::require_admin(&env, &admin)?;
        if price <= 0 { return Err(AjoError::InvalidInput); }
        env.storage().instance().set(&DataKey::EthUsdPrice, &price);
        env.storage().instance().set(&DataKey::EthUsdDecimals, &decimals);
        Ok(())
    }

    pub fn native_amount_for_usd(env: Env, usd_amount: i128) -> Result<i128, AjoError> {
        if usd_amount <= 0 { return Err(AjoError::InvalidInput); }
        let price: i128 = env.storage().instance().get(&DataKey::EthUsdPrice).ok_or(AjoError::PriceUnavailable)?;
        let decimals: u32 = env.storage().instance().get(&DataKey::EthUsdDecimals).ok_or(AjoError::PriceUnavailable)?;
        let scale = Self::pow10_checked(decimals)?;
        let native = usd_amount.checked_mul(scale).ok_or(AjoError::ArithmeticOverflow)?.checked_div(price).ok_or(AjoError::ArithmeticOverflow)?;
        if native <= 0 { return Err(AjoError::InvalidInput); }
        Ok(native)
    }

    pub fn get_total_pool(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalPool).unwrap_or(0)
    }

    pub fn get_last_deposit_timestamp(env: Env, member: Address) -> Result<u64, AjoError> {
        let m: Map<Address, u64> = env.storage().instance().get(&DataKey::LastDepositAt).ok_or(AjoError::NotFound)?;
        m.get(member).ok_or(AjoError::NotFound)
    }

    pub fn shuffle_rotation(env: Env, organizer: Address) -> Result<(), AjoError> {
        organizer.require_auth();
        let circle: CircleData = env.storage().instance().get(&DataKey::Circle).ok_or(AjoError::NotFound)?;
        if circle.organizer != organizer { return Err(AjoError::Unauthorized); }
        if Self::get_circle_status(env.clone()) == CircleStatus::Panicked { return Err(AjoError::CirclePanicked); }

        let members: Map<Address, MemberData> = env.storage().instance().get(&DataKey::Members).ok_or(AjoError::NotFound)?;
        let mut rotation: Vec<Address> = Vec::new(&env);
        for (addr, _) in members.iter() { rotation.push_back(addr); }

        let n = rotation.len();
        if n < 2 {
            env.storage().instance().set(&DataKey::RotationOrder, &rotation);
            return Ok(());
        }

        let ledger_seq = env.ledger().sequence();
        let tx_hash: BytesN<32> = env.crypto().sha256(&soroban_sdk::Bytes::from_slice(&env, &ledger_seq.to_be_bytes())).into();
        let hash_bytes = tx_hash.to_array();

        for i in (1..n).rev() {
            let byte_idx = (i as usize) % 32;
            let j = (hash_bytes[byte_idx] as u32) % (i + 1);
            let a = rotation.get(i).unwrap();
            let b = rotation.get(j).unwrap();
            rotation.set(i, b);
            rotation.set(j, a);
        }
        env.storage().instance().set(&DataKey::RotationOrder, &rotation);
        Ok(())
    }

    pub fn slash_member(env: Env, admin: Address, member: Address) -> Result<(), AjoError> {
        Self::require_admin(&env, &admin)?;
        let mut standings: Map<Address, MemberStanding> = env.storage().instance().get(&DataKey::Standings).unwrap_or(Map::new(&env));
        if let Some(mut standing) = standings.get(member.clone()) {
            standing.missed_count += 1;
            if standing.missed_count >= 3 { standing.is_active = false; }
            standings.set(member.clone(), standing);
            env.storage().instance().set(&DataKey::Standings, &standings);
            Ok(())
        } else {
            Err(AjoError::NotFound)
        }
    }

    pub fn boot_dormant_member(env: Env, admin: Address, member: Address) -> Result<(), AjoError> {
        Self::require_admin(&env, &admin)?;
        let mut standings: Map<Address, MemberStanding> = env.storage().instance().get(&DataKey::Standings).unwrap_or(Map::new(&env));
        if let Some(mut standing) = standings.get(member.clone()) {
            standing.is_active = false;
            standings.set(member.clone(), standing);
        } else {
            return Err(AjoError::NotFound);
        }

        let mut members: Map<Address, MemberData> = env.storage().instance().get(&DataKey::Members).ok_or(AjoError::NotFound)?;
        if let Some(mut member_data) = members.get(member.clone()) {
            member_data.status = 2;
            members.set(member, member_data);
        } else {
            return Err(AjoError::NotFound);
        }

        env.storage().instance().set(&DataKey::Standings, &standings);
        env.storage().instance().set(&DataKey::Members, &members);
        Ok(())
    }

    pub fn upgrade(env: Env, admin: Address, new_wasm_hash: BytesN<32>) -> Result<(), AjoError> {
        Self::require_admin(&env, &admin)?;
        env.deployer().update_current_contract_wasm(new_wasm_hash);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env, token};

    fn setup_env() -> Env {
        Env::default()
    }

    fn setup_circle_with_member(env: &Env) -> (AjoCircleClient<'_>, Address, Address, Address) {
        let contract_id = env.register_contract(None, AjoCircle);
        let client = AjoCircleClient::new(env, &contract_id);

        let organizer = Address::generate(env);
        let member = Address::generate(env);
        let token_admin = Address::generate(env);
        let token_address = env.register_stellar_asset_contract(token_admin.clone());
        let token_sa_client = token::StellarAssetClient::new(env, &token_address);

        token_sa_client.mint(&organizer, &1000_i128);
        token_sa_client.mint(&member, &1000_i128);

        client.initialize_circle(&organizer, &token_address, &100_i128, &7_u32, &12_u32, &5_u32);
        client.add_member(&organizer, &member);
        
        (client, organizer, member, token_address)
    }

    #[test]
    fn enforce_member_limit() {
        let env = setup_env();
        env.mock_all_auths();
        let (client, organizer, _, token_address) = setup_circle_with_member(&env);
        
        // Setup circle with limit 2 (organizer + 1 member)
        client.initialize_circle(&organizer, &token_address, &100_i128, &7_u32, &12_u32, &2_u32);
        client.add_member(&organizer, &Address::generate(&env));
        let res = client.add_member(&organizer, &Address::generate(&env));
        assert_eq!(res, Err(AjoError::CircleAtCapacity));
    }

    #[test]
    fn test_panic_flow() {
        let env = setup_env();
        env.mock_all_auths();
        let (client, organizer, member, _) = setup_circle_with_member(&env);

        assert!(!client.is_panicked());
        client.panic(&organizer).unwrap();
        assert!(client.is_panicked());
        assert_eq!(client.get_circle_status(), CircleStatus::Panicked);

        let res = client.contribute(&member, &50_i128);
        assert_eq!(res, Err(AjoError::CirclePanicked));
    }

    #[test]
    fn test_emergency_refund() {
        let env = setup_env();
        env.mock_all_auths();
        let (client, organizer, member, token_address) = setup_circle_with_member(&env);
        let token_client = token::Client::new(&env, &token_address);

        client.contribute(&member, &200_i128).unwrap();
        assert_eq!(token_client.balance(&member), 800_i128);

        client.panic(&organizer).unwrap();
        let refund = client.emergency_refund(&member).unwrap();
        assert_eq!(refund, 200_i128);
        assert_eq!(token_client.balance(&member), 1000_i128);
    }
}
