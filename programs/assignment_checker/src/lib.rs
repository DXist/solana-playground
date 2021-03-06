use anchor_lang::prelude::*;
use anchor_lang::solana_program::blake3;

declare_id!("Po1RaS8BEDbNcn5oXsFryAeQ6Wn8fvmE111DJaKCgPC");

pub const COURSE_DATA_SEED: &[u8; 11] = b"course_data";
pub const ASSIGNMENT_ID_SEED: &[u8; 13] = b"assignment_id";
pub const STUDENT_ADDRESS_SEED: &[u8; 15] = b"student_address";
// Owner of AssignmentCheckerState and CheckResult accounts
#[program]
pub mod assignment_checker {
    use super::*;

    pub fn init(
        ctx: Context<Init>,
        assignment_id: [u8; 16],
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
        checker_account.assignment_id = assignment_id;
        checker_account.hash_chain_length = hash_chain_length;
        checker_account.to_mint_on_successful_check = to_mint_on_successful_check;
        checker_account.salt = salt;
        *checker_account.ground_truth_hash_chain_tail() = ground_truth_hash_chain_tail;
        checker_account.bump_seed = *ctx
            .bumps
            .get("assignment_checker")
            .expect("assignment_checker pda is present");
        msg!("init assignment checker account {}", checker_account.key(),);
        Ok(())
    }

    /// Init check result created by the result_processor_program
    pub fn init_check_result(ctx: Context<InitCheckResult>, assignment_id: [u8; 16]) -> Result<()> {
        let check_result = &mut ctx.accounts.check_result;
        check_result.assignment_id = assignment_id;
        check_result.bump_seed = *ctx
            .bumps
            .get("check_result")
            .expect("check_result pda is present");
        msg!("init check result account {}", check_result.key());
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
                msg!("check is passed");
            }
            // else: keep check_passed and passed_first_time as false
        }
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(assignment_id: [u8; 16], hash_chain_length: u16)]
pub struct Init<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(has_one = authority)]
    pub course: Account<'info, course_manager::Course>,

    #[account(
        zero,
        signer,
        seeds=[
        COURSE_DATA_SEED,
        course.key().as_ref(),
        ASSIGNMENT_ID_SEED,
        assignment_id.as_ref(),
    ],
    seeds::program = result_processor_program, bump, constraint = hash_chain_length >= 2)]
    pub assignment_checker: Account<'info, AssignmentCheckerState>,
    #[account(executable)]
    pub result_processor_program: AccountInfo<'info>,
}

#[derive(Accounts)]
#[instruction(assignment_id: [u8; 16])]
pub struct InitCheckResult<'info> {
    #[account(mut)]
    pub student: Signer<'info>,
    pub course: Account<'info, course_manager::Course>,

    #[account(zero,
        signer,
        seeds=[
        STUDENT_ADDRESS_SEED,
        student.key().as_ref(),
        COURSE_DATA_SEED,
        course.key().as_ref(),
        ASSIGNMENT_ID_SEED,
        assignment_id.as_ref(),
    ],
    seeds::program = result_processor_program,
    bump)]
    pub check_result: Account<'info, CheckResult>,
    #[account(executable)]
    pub result_processor_program: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct Check<'info> {
    #[account(mut)]
    pub student: Signer<'info>,
    pub course: Account<'info, course_manager::Course>,

    #[account(mut,
        signer,
        seeds=[
        COURSE_DATA_SEED,
        course.key().as_ref(),
        ASSIGNMENT_ID_SEED,
        assignment_checker.assignment_id.as_ref(),
    ], seeds::program = result_processor_program, bump=assignment_checker.bump_seed,
    constraint = assignment_checker.assignment_id == check_result.assignment_id
    )]
    pub assignment_checker: Account<'info, AssignmentCheckerState>,

    #[account(mut,
        signer,
        seeds=[
        STUDENT_ADDRESS_SEED,
        student.key().as_ref(),
        COURSE_DATA_SEED,
        course.key().as_ref(),
        ASSIGNMENT_ID_SEED,
        check_result.assignment_id.as_ref(),
    ], seeds::program = result_processor_program, bump=check_result.bump_seed,
    )]
    pub check_result: Account<'info, CheckResult>,
    // result_processor_program is expected to be called by student
    // and sign for mutable PDAs
    // student cannot call check directly
    #[account(executable)]
    pub result_processor_program: AccountInfo<'info>,
}

#[account]
pub struct AssignmentCheckerState {
    /// Assignment ID is unique within a course
    pub assignment_id: [u8; 16],
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

impl AssignmentCheckerState {
    pub const LEN: usize = 16 + 2 + 2 + 32 + 32 + 1;

    pub fn ground_truth_hash_chain_tail(&mut self) -> &mut [u8; 32] {
        &mut self.ground_truth_hash_chain_tail
    }
}

#[account]
pub struct CheckResult {
    /// Assignment ID is unique within a course
    pub assignment_id: [u8; 16],
    pub check_passed: bool,
    /// This is true only after first successful check
    pub passed_first_time: bool,
    pub bump_seed: u8,
}

impl CheckResult {
    pub const LEN: usize = 16 + 1 + 1 + 1;
}

#[error_code]
pub enum AssignmentCheckerError {
    #[msg("The hash chain for this checker is fully used")]
    ZeroHashChainLength,
    #[msg("The hash chain for this checker differs from provided expected hash chain length. Retry with updated expected length.")]
    ExpectedHashLengthDiffers,
}
