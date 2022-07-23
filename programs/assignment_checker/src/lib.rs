use anchor_lang::prelude::*;
use anchor_lang::solana_program::blake3;

pub const STUDENT_ACCOUNT_SEED: &[u8; 15] = b"student_account";
pub const COURSE_ACCOUNT_SEED: &[u8; 14] = b"course_account";
pub const BATCH_ID_SEED: &[u8; 8] = b"batch_id";
pub const ASSIGNMENT_ID_SEED: &[u8; 13] = b"assignment_id";

declare_id!("Po1RaS8BEDbNcn5oXsFryAeQ6Wn8fvmE111DJaKCgPC");
#[program]
pub mod assignment_checker {
    use super::*;

    /// Create an assignment checker
    pub fn create(
        ctx: Context<Create>,
        batch_id: u16,
        assignment_id: u16,
        hash_chain_length: u16,
        to_mint_on_successful_check: u16,
        salt: [u8; 32],
        // Creator of assignment checker is a trusted authority
        // It should precompute ground truth hash chain tail
        // to save nonfree compute operations of onchain program
        // and not to send the ground truth assignment result value to public blockchain
        ground_truth_hash_chain_tail: [u8; 32],
    ) -> Result<()> {
        let checker_account = &mut ctx.accounts.assignment_checker;
        checker_account.batch_id = batch_id;
        checker_account.assignment_id = assignment_id;
        checker_account.hash_chain_length = hash_chain_length;
        checker_account.to_mint_on_successful_check = to_mint_on_successful_check;
        checker_account.salt = salt;
        checker_account.ground_truth_hash_chain_tail = ground_truth_hash_chain_tail;
        checker_account.bump_seed = *ctx
            .bumps
            .get("assignment_checker")
            .expect("assignment_checker pda is present");
        Ok(())
    }

    /// Check assignment and save result into check_result account.
    ///
    /// Errors:
    ///     * Returns `AssignmentChecker::ZeroHashChainLength` when the hash
    ///     chain is fully used.
    ///     * Returns `AssignmentChecker::ExpectedHashLengthDiffers` when client expects
    ///     different hash chain length than the checker currently has.  This
    ///     can happen during concurrent checks by multiple students and should
    ///     be mitigated by retry with actual hash chain length
    pub fn check(
        ctx: Context<Check>,
        batch_id: u16,
        assignment_id: u16,
        // used to validate the hash chain length
        // that the client expects and deal with concurrent checks
        expected_hash_chain_length: u16,
        // the hash before current hash chain tail
        hash_chain_tail_parent: [u8; 32],
    ) -> Result<()> {
        let check_result_account = &mut ctx.accounts.check_result;
        if check_result_account.check_passed {
            // previous check succeded
            // This check is no longer the first
            check_result_account.passed_first_time = false;
        } else {
            // this check hasn't passed yet
            // save bump seed for future validations
            check_result_account.bump_seed = *ctx
                .bumps
                .get("check_result")
                .expect("check_result pda is present");

            let checker_account = &mut ctx.accounts.assignment_checker;
            if checker_account.hash_chain_length == 0 {
                // checker has used full hash chain
                return Err(error!(AssignmentCheckerError::ZeroHashChainLength));
            }

            if checker_account.hash_chain_length != expected_hash_chain_length {
                // client expects different hash chain length then the checker has at the moment
                return Err(error!(AssignmentCheckerError::ExpectedHashLengthDiffers));
            }

            let tail_hash = blake3::hash(&hash_chain_tail_parent);
            if tail_hash == blake3::Hash(checker_account.ground_truth_hash_chain_tail) {
                // check has passed the first time
                check_result_account.check_passed = true;
                check_result_account.passed_first_time = true;
                // remove tail from the chain
                checker_account.hash_chain_length -= 1;
                checker_account.ground_truth_hash_chain_tail = hash_chain_tail_parent;
            }
            // else: check_result is zero initialized => check_passed is false
        }
        Ok(())
    }
}

// validation struct for Create instruction
#[derive(Accounts)]
#[instruction(batch_id: u16, assignment_id: u16, hash_chain_length: u16)]
pub struct Create<'info> {
    #[account(mut)]
    pub course_authority: Signer<'info>,
    pub course_account: Account<'info, course_manager::Course>,
    // By default init sets the owner field of the created account to the currently executing program.
    // Anchor will find the canonical bump for the assignment checker PDA.
    // The PDA is derived from course account, batch and assignment IDs.
    #[account(init, payer = course_authority, space = 8 + AssignmentChecker::LEN, seeds=[
        COURSE_ACCOUNT_SEED,
        course_account.key().as_ref(),
        BATCH_ID_SEED,
        batch_id.to_be_bytes().as_ref(),
        ASSIGNMENT_ID_SEED,
        assignment_id.to_be_bytes().as_ref(),
    ], bump, constraint = hash_chain_length >= 2)]
    pub assignment_checker: Account<'info, AssignmentChecker>,
    // #[account(
    //     mint::authority = course_authority,
    // )]
    // pub mint_account: Account<'info, Mint>,
    pub system_program: Program<'info, System>,
}

// validation struct for Create instruction
#[derive(Accounts)]
#[instruction(batch_id: u16, assignment_id: u16)]
pub struct Check<'info> {
    #[account(mut)]
    pub student: Signer<'info>,
    pub course_account: Account<'info, course_manager::Course>,

    #[account(mut, seeds=[
        COURSE_ACCOUNT_SEED,
        course_account.key().as_ref(),
        BATCH_ID_SEED,
        batch_id.to_be_bytes().as_ref(),
        ASSIGNMENT_ID_SEED,
        assignment_id.to_be_bytes().as_ref(),
    ], bump = assignment_checker.bump_seed)]
    pub assignment_checker: Account<'info, AssignmentChecker>,

    #[account(init_if_needed, payer = student, space = 8 + AssignmentChecker::LEN, seeds=[
        STUDENT_ACCOUNT_SEED,
        student.key().as_ref(),
        COURSE_ACCOUNT_SEED,
        course_account.key().as_ref(),
        BATCH_ID_SEED,
        batch_id.to_be_bytes().as_ref(),
        ASSIGNMENT_ID_SEED,
        assignment_id.to_be_bytes().as_ref(),
    ], bump)]
    pub check_result: Account<'info, CheckResult>,
    pub system_program: Program<'info, System>,
}

#[account]
pub struct AssignmentChecker {
    /// Batch identifies the given course run
    pub batch_id: u16,
    /// Assignment ID is unique within a course batch
    pub assignment_id: u16,
    /// Max number of successful checks possible + 1
    ///
    /// at least 1 check per student of the batch + 1 hash
    /// to keep the ground truth value away of sending to public blockchain
    pub hash_chain_length: u16,
    pub to_mint_on_successful_check: u16,
    pub salt: [u8; 32],
    /// Result of hash(...(hash(hashv([salt, value]))...)
    ///
    /// hash is applied `hash_chain_length` number of times
    ground_truth_hash_chain_tail: [u8; 32],
    pub bump_seed: u8,
}

impl AssignmentChecker {
    pub const LEN: usize = 2 + 2 + 2 + 2 + 32 + 32 + 1;
}

#[account]
pub struct CheckResult {
    /// Batch identifies the given course run
    pub batch_id: u16,
    /// Assignment ID is unique within a course batch
    pub assignment_id: u16,
    pub check_passed: bool,
    /// This is true only after first successful check
    pub passed_first_time: bool,
    pub bump_seed: u8,
}

impl CheckResult {
    pub const LEN: usize = 2 + 2 + 1 + 1;
}

#[error_code]
pub enum AssignmentCheckerError {
    #[msg("The hash chain for this checker is fully used")]
    ZeroHashChainLength,
    #[msg("The hash chain for this checker differs from provided expected hash chain length. Retry with updated expected length.")]
    ExpectedHashLengthDiffers,
}
