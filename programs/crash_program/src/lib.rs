use anchor_lang::system_program;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    ed25519_program,
    program::{invoke, invoke_signed},
    system_instruction,
    sysvar::instructions::{
        load_current_index_checked,
        load_instruction_at_checked,
        ID as SYSVAR_INSTRUCTIONS_ID,
    },
};

declare_id!("7FydhAeaHUrwkhRkPoSUi4AHedQEGdn5gBwfzHyUBtAj"); // <-- replace after deploy

const MAX_PAYOUT_LAMPORTS: u64 = 50_000_000_000; // 0.05 SOL
const MIN_BET_LAMPORTS: u64 = 50_000;            // 0.00005 SOL
const MAX_BET_LAMPORTS: u64 = 5_000_000_000;     // 5 SOL
const MIN_MULT_BPS: u32 = 10_000;                // 1.00x
const MAX_MULT_BPS: u32 = 1_000_000;             // 100.00x

#[error_code]
pub enum CrashError {
    #[msg("Invalid ed25519 pre-instruction")] InvalidEd25519,
    #[msg("Expired signature")] Expired,
    #[msg("Params invalid")] BadParams,
    #[msg("Payout sanity check failed")] BadPayout,
    #[msg("Vault mismatch")] VaultMismatch,
    #[msg("Round not found or already settled")] BadRound,
}

#[account]
pub struct AdminConfig {
    /// Trusted ed25519 public key (32 bytes) of your backend signer
    pub admin_pubkey: [u8; 32],
}

#[account]
pub struct PendingRound {
    pub player: Pubkey,
    pub amount: u64,
    pub nonce: u64,
    pub expiry_unix: i64,
    pub settled: bool,
}
impl PendingRound {
    pub const LEN: usize = 8 + 32 + 8 + 8 + 8 + 1;
}

#[derive(Accounts)]
pub struct InitAdmin<'info> {
    #[account(mut, signer)]
    pub authority: SystemAccount<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + 32,
        seeds = [b"admin"],
        bump
    )]
    pub admin_config: Account<'info, AdminConfig>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct InitVault<'info> {
    #[account(mut, signer)]
    pub payer: SystemAccount<'info>,

    /// CHECK: This vault PDA is created (or must already exist) using the static seed/bump pair.
    /// It will be a System Program-owned account that only holds lamports for payouts.
    /// We do not deserialize structured data from it, and the seeds constraint enforces PDA derivation.
    #[account(mut, seeds = [b"vault"], bump)]
    pub vault: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(args: LockArgs)]
pub struct Lock<'info> {
    #[account(mut, signer)]
    pub player: SystemAccount<'info>,

    #[account(mut, seeds = [b"vault"], bump)]
    pub vault: SystemAccount<'info>,

    #[account(
        init,
        payer = player,
        space = PendingRound::LEN,
        seeds = [b"round", player.key().as_ref(), &args.nonce.to_le_bytes()],
        bump
    )]
    pub pending_round: Account<'info, PendingRound>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Resolve<'info> {
    #[account(mut)]
    pub player: SystemAccount<'info>,

    #[account(mut, seeds = [b"vault"], bump)]
    pub vault: SystemAccount<'info>,

    /// Admin config PDA containing backend ed25519 pubkey
    #[account(seeds = [b"admin"], bump)]
    pub admin_config: Account<'info, AdminConfig>,

    #[account(
        mut,
        seeds = [b"round", player.key().as_ref(), &pending_round.nonce.to_le_bytes()],
        bump,
        close = player
    )]
    pub pending_round: Account<'info, PendingRound>,

    pub system_program: Program<'info, System>,

    /// CHECK: This is the sysvar instructions account supplied by the runtime.
    /// We constrain the exact address using `address = SYSVAR_INSTRUCTIONS_ID`.
    /// It is safe to treat as UncheckedAccount because we only use it to load instructions.
    #[account(address = SYSVAR_INSTRUCTIONS_ID)]
    pub sysvar_instructions: UncheckedAccount<'info>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct LockArgs {
    pub bet_amount: u64,
    pub nonce: u64,
    pub expiry_unix: i64,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct ResolveArgs {
    pub checksum: u8,              // arbitrary 1..100
    pub multiplier_bps: u32,       // 1.00x = 10000, 2.34x = 23400
    pub payout: u64,               // net (gross - principal). 0 on crash
    pub ed25519_instr_index: u8,   // index hint of ed25519 verify ix
}

#[event]
pub struct RoundLocked {
    pub player: Pubkey,
    pub amount: u64,
    pub nonce: u64,
}

#[event]
pub struct RoundResolved {
    pub player: Pubkey,
    pub cashed: bool,
    pub checksum: u8,
    pub multiplier_bps: u32,
    pub payout: u64,
    pub nonce: u64,
}

#[program]
pub mod anchor_crash {
    use super::*;

    pub fn init_admin(ctx: Context<InitAdmin>, admin_pubkey: [u8; 32]) -> Result<()> {
        ctx.accounts.admin_config.admin_pubkey = admin_pubkey;
        Ok(())
    }

    pub fn init_vault(ctx: Context<InitVault>) -> Result<()> {
        // Create the vault PDA as a system account
        let rent = Rent::get()?.minimum_balance(0);
        let lamports = rent.max(1);
        let bump = ctx.bumps.vault;

        let create_ix = system_instruction::create_account(
            &ctx.accounts.payer.key(),
            &ctx.accounts.vault.key(),
            lamports,
            0,
            &system_program::ID,
        );

        invoke_signed(
            &create_ix,
            &[
                ctx.accounts.payer.to_account_info(),
                ctx.accounts.vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[&[b"vault", &[bump]]],
        )?;

        Ok(())
    }

    /// Step 1: Player deposits bet into vault and opens a PendingRound
    pub fn lock(ctx: Context<Lock>, args: LockArgs) -> Result<()> {
        require!(
            args.bet_amount >= MIN_BET_LAMPORTS && args.bet_amount <= MAX_BET_LAMPORTS,
            CrashError::BadParams
        );

        // Transfer player → vault
        let collect_ix = system_instruction::transfer(
            &ctx.accounts.player.key(),
            &ctx.accounts.vault.key(),
            args.bet_amount,
        );
        invoke(
            &collect_ix,
            &[
                ctx.accounts.player.to_account_info(),
                ctx.accounts.vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // Record pending round
        let pr = &mut ctx.accounts.pending_round;
        pr.player = ctx.accounts.player.key();
        pr.amount = args.bet_amount;
        pr.nonce = args.nonce;
        pr.expiry_unix = args.expiry_unix;
        pr.settled = false;

        emit!(RoundLocked {
            player: pr.player,
            amount: pr.amount,
            nonce: pr.nonce,
        });

        Ok(())
    }

    /// Step 2: Backend signs result. Program verifies pre-instruction + rails, then pays if cashed
    pub fn resolve(ctx: Context<Resolve>, args: ResolveArgs) -> Result<()> {
        let pr = &mut ctx.accounts.pending_round;
        require!(!pr.settled, CrashError::BadRound);

        // Expiry
        let clock = Clock::get()?;
        require!(clock.unix_timestamp <= pr.expiry_unix, CrashError::Expired);

        // --- Ed25519 presence check (index hint + fallback scan) ---
        let sys_ix_ai = &ctx.accounts.sysvar_instructions.to_account_info();

        let hinted_ok = load_instruction_at_checked(args.ed25519_instr_index as usize, sys_ix_ai)
            .map(|ix| ix.program_id == ed25519_program::id())
            .unwrap_or(false);

        let mut found = hinted_ok;
        if !found {
            let cur_idx = load_current_index_checked(sys_ix_ai)?;
            for i in 0..cur_idx {
                if let Ok(ix) = load_instruction_at_checked(i as usize, sys_ix_ai) {
                    if ix.program_id == ed25519_program::id() {
                        found = true;
                        break;
                    }
                }
            }
        }
        require!(found, CrashError::InvalidEd25519);

        // Rails for Crash
        require!(args.multiplier_bps >= MIN_MULT_BPS && args.multiplier_bps <= MAX_MULT_BPS, CrashError::BadParams);
        require!(args.checksum >= 1 && args.checksum <= 100, CrashError::BadParams);

        // Expected net payout based on multiplier: floor(amount * m_bps / 10000) - amount
        let gross = (pr.amount as u128) * (args.multiplier_bps as u128) / 10_000u128;
        let expected_net = if gross > pr.amount as u128 {
            (gross - pr.amount as u128) as u64
        } else {
            0u64
        };

        // Match provided payout and safety cap
        if expected_net > 0 {
            require!(args.payout == expected_net, CrashError::BadPayout);
            require!(args.payout <= MAX_PAYOUT_LAMPORTS, CrashError::BadPayout);
        } else {
            require!(args.payout == 0, CrashError::BadPayout);
        }

        // Pay winnings from vault → player
        if args.payout > 0 {
            let payout_ix = system_instruction::transfer(
                &ctx.accounts.vault.key(),
                &ctx.accounts.player.key(),
                args.payout,
            );
            let bump = ctx.bumps.vault;
            let seeds: &[&[u8]] = &[b"vault", &[bump]];
            invoke_signed(
                &payout_ix,
                &[
                    ctx.accounts.vault.to_account_info(),
                    ctx.accounts.player.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
                &[seeds],
            )?;
        }

        // Mark settled (account closes to player at end of ix due to `close = player`)
        pr.settled = true;

        emit!(RoundResolved {
            player: pr.player,
            cashed: expected_net > 0,
            checksum: args.checksum,
            multiplier_bps: args.multiplier_bps,
            payout: args.payout,
            nonce: pr.nonce,
        });

        Ok(())
    }
}
