use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_lang::solana_program::{
    ed25519_program,
    instruction::Instruction,
    program::{invoke, invoke_signed},
    system_instruction,
    sysvar::instructions::{load_current_index_checked, load_instruction_at_checked, ID as SYSVAR_INSTRUCTIONS_ID},
};

declare_id!("5rSyAdhghTNwSzVU7PxJwJY7omPGxvQjdtfyn4BxJABr");

// Canonical domain tag for off-chain signing
const DOMAIN_TAG: &[u8] = b"PLINKO_V1";

// Rails / caps
const MIN_BET_LAMPORTS: u64 = 50_000;
const MAX_BET_LAMPORTS: u64 = 10_000_000_000_000;
const MAX_PAYOUT_LAMPORTS: u64 = 20_000_000_000_000;
const MIN_ROWS: u8 = 8;
const MAX_ROWS: u8 = 16;
const MAX_BALLS: u16 = 10_000;

#[error_code]
pub enum PlinkoError {
    #[msg("Invalid instruction data")] InvalidIx,
    #[msg("Invalid ed25519 pre-instruction")] InvalidEd25519,
    #[msg("Expired signature")] Expired,
    #[msg("Bad params")] BadParams,
    #[msg("Payout sanity check failed")] BadPayout,
}

#[account]
pub struct AdminConfig {
    pub admin_pubkey: [u8; 32],
}

#[account]
pub struct PendingRound {
    pub player: Pubkey,
    pub unit_amount: u64,   // per ball
    pub balls: u16,
    pub rows: u8,
    pub difficulty: u8,     // 0..4
    pub nonce: u64,
    pub expiry_unix: i64,
    pub settled: bool,
}
impl PendingRound {
    pub const LEN: usize = 8 + 32 + 8 + 2 + 1 + 1 + 8 + 8 + 1;
}

#[derive(Accounts)]
pub struct InitAdmin <'info> {
    #[account(mut, signer)]
    pub authority: SystemAccount<'info>,
    #[account(init, payer = authority, space = 8 + 32, seeds = [b"admin"], bump)]
    pub admin_config: Account<'info, AdminConfig>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct InitVault<'info> {
    #[account(mut, signer)]
    pub payer: SystemAccount<'info>,

    /// CHECK: created as system account via invoke_signed
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
        seeds = [b"bet", player.key().as_ref(), &args.nonce.to_le_bytes()],
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

    #[account(seeds = [b"admin"], bump)]
    pub admin_config: Account<'info, AdminConfig>,

    #[account(
        mut,
        seeds = [b"bet", player.key().as_ref(), &pending_round.nonce.to_le_bytes()],
        bump,
        close = player
    )]
    pub pending_round: Account<'info, PendingRound>,

    pub system_program: Program<'info, System>,

    /// CHECK: sysvar instructions
    #[account(address = SYSVAR_INSTRUCTIONS_ID)]
    pub sysvar_instructions: UncheckedAccount<'info>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct LockArgs {
    pub unit_amount: u64,
    pub balls: u16,
    pub rows: u8,
    pub difficulty: u8, // 0..4
    pub nonce: u64,
    pub expiry_unix: i64,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct ResolveArgs {
    pub checksum: u8,
    pub payout: u64,             // NET payout (profit-only)
    pub ed25519_instr_index: u8, // index hint
}

#[event]
pub struct PlinkoLocked {
    pub player: Pubkey,
    pub unit_amount: u64,
    pub balls: u16,
    pub rows: u8,
    pub difficulty: u8,
    pub nonce: u64,
}

#[event]
pub struct PlinkoResolved {
    pub player: Pubkey,
    pub payout: u64,
    pub nonce: u64,
}

#[program]
pub mod plinko_program {
    use super::*;

    pub fn init_admin(ctx: Context<InitAdmin>, admin_pubkey: [u8; 32]) -> Result<()> {
        ctx.accounts.admin_config.admin_pubkey = admin_pubkey;
        Ok(())
    }

    pub fn init_vault(ctx: Context<InitVault>) -> Result<()> {
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

    pub fn lock(ctx: Context<Lock>, args: LockArgs) -> Result<()> {
        require!(args.rows >= MIN_ROWS && args.rows <= MAX_ROWS, PlinkoError::BadParams);
        require!(args.difficulty <= 4, PlinkoError::BadParams);
        require!(args.balls >= 1 && args.balls <= MAX_BALLS, PlinkoError::BadParams);
        require!(args.unit_amount >= MIN_BET_LAMPORTS, PlinkoError::BadParams);

        let total = (args.unit_amount as u128)
            .checked_mul(args.balls as u128)
            .ok_or(PlinkoError::BadParams)?;
        require!(total <= MAX_BET_LAMPORTS as u128, PlinkoError::BadParams);

        // Transfer player → vault
        let collect_ix = system_instruction::transfer(
            &ctx.accounts.player.key(),
            &ctx.accounts.vault.key(),
            total as u64,
        );
        invoke(
            &collect_ix,
            &[
                ctx.accounts.player.to_account_info(),
                ctx.accounts.vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // Record pending
        let pr = &mut ctx.accounts.pending_round;
        pr.player = ctx.accounts.player.key();
        pr.unit_amount = args.unit_amount;
        pr.balls = args.balls;
        pr.rows = args.rows;
        pr.difficulty = args.difficulty;
        pr.nonce = args.nonce;
        pr.expiry_unix = args.expiry_unix;
        pr.settled = false;

        emit!(PlinkoLocked {
            player: pr.player,
            unit_amount: pr.unit_amount,
            balls: pr.balls,
            rows: pr.rows,
            difficulty: pr.difficulty,
            nonce: pr.nonce,
        });

        Ok(())
    }

   pub fn resolve(ctx: Context<Resolve>, args: ResolveArgs) -> Result<()> {
    // take immutable data you'll need *before* mutable borrow
    let pending_key = ctx.accounts.pending_round.key();
    let vault_key   = ctx.accounts.vault.key();
    let player_key  = ctx.accounts.player.key();

    // now the mutable borrow
    let pr = &mut ctx.accounts.pending_round;
    require!(!pr.settled, PlinkoError::BadParams);

    // 1) expiry
    let clock = Clock::get()?;
    require!(clock.unix_timestamp <= pr.expiry_unix, PlinkoError::Expired);

    // 2) find Ed25519 pre-ix (unchanged) ...
    let sys_ai = &ctx.accounts.sysvar_instructions.to_account_info();
    let mut target_ix: Option<Instruction> = None;
    if let Ok(ix) = load_instruction_at_checked(args.ed25519_instr_index as usize, sys_ai) {
        if ix.program_id == ed25519_program::id() { target_ix = Some(ix); }
    }
    if target_ix.is_none() {
        let cur = load_current_index_checked(sys_ai)?;
        for i in 0..cur {
            if let Ok(ix) = load_instruction_at_checked(i as usize, sys_ai) {
                if ix.program_id == ed25519_program::id() {
                    target_ix = Some(ix);
                    break;
                }
            }
        }
    }
    require!(target_ix.is_some(), PlinkoError::InvalidEd25519);
    let ed_ix = target_ix.unwrap();

    // 3) extract signed msg & pubkey and compare
    let (signed_msg, signed_pk) = extract_ed25519_msg_and_pubkey(&ed_ix)?;
    require!(signed_pk == ctx.accounts.admin_config.admin_pubkey, PlinkoError::InvalidEd25519);

    let expected = build_canonical_msg(
        ctx.program_id,
        &vault_key,
        &player_key,
        &pending_key,
        pr,              // OK: we already captured the keys above
        args.payout,
    );
    require!(signed_msg == expected.as_slice(), PlinkoError::InvalidEd25519);

    // 4) payout rails — sanity cap
    if args.payout > 0 {
        require!(args.payout <= MAX_PAYOUT_LAMPORTS, PlinkoError::BadPayout);
    }

    // 5) Transfer principal + net payout
    let principal = (pr.unit_amount as u128)
        .checked_mul(pr.balls as u128)
        .ok_or(PlinkoError::BadPayout)? as u64;
    let total_out = principal.saturating_add(args.payout);
    if total_out > 0 {
        let bump = ctx.bumps.vault;
        let ix = system_instruction::transfer(&vault_key, &player_key, total_out);
        invoke_signed(
            &ix,
            &[
                ctx.accounts.vault.to_account_info(),
                ctx.accounts.player.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[&[b"vault", &[bump]]],
        )?;
    }

    pr.settled = true;
    emit!(PlinkoResolved { player: pr.player, payout: args.payout, nonce: pr.nonce });
    Ok(())
}
}

// === helpers to parse ed25519 pre-ix and build canonical message ===
fn rd_u16(d: &[u8], off: usize) -> Result<u16> {
    require!(off + 2 <= d.len(), PlinkoError::InvalidEd25519);
    Ok(u16::from_le_bytes([d[off], d[off + 1]]))
}
fn extract_ed25519_msg_and_pubkey(ix: &Instruction) -> Result<(&[u8], &[u8])> {
    let d = &ix.data;
    require!(d.len() >= 16, PlinkoError::InvalidEd25519);
    let num = d[0] as usize;
    require!(num == 1, PlinkoError::InvalidEd25519);

    // offsets: sig_off u16 [2], sig_ix u16 [4], pk_off u16 [6], pk_ix u16 [8],
    // msg_off u16 [10], msg_sz u16 [12], msg_ix u16 [14]
    let pk_off  = rd_u16(d, 6)? as usize;
    let msg_off = rd_u16(d,10)? as usize;
    let msg_sz  = rd_u16(d,12)? as usize;

    require!(pk_off + 32 <= d.len(), PlinkoError::InvalidEd25519);
    require!(msg_off + msg_sz <= d.len(), PlinkoError::InvalidEd25519);

    let pubkey = &d[pk_off .. pk_off + 32];
    let msg    = &d[msg_off .. msg_off + msg_sz];
    Ok((msg, pubkey))
}
fn build_canonical_msg(
    program_id: &Pubkey,
    vault: &Pubkey,
    player: &Pubkey,
    pending: &Pubkey,
    pr: &PendingRound,
    payout: u64,
) -> Vec<u8> {
    let mut v = Vec::with_capacity(1 + 32*4 + 8 + 4 + 1 + 1 + 8 + 8 + 8);
    v.extend_from_slice(DOMAIN_TAG);
    v.extend_from_slice(program_id.as_ref());
    v.extend_from_slice(vault.as_ref());
    v.extend_from_slice(player.as_ref());
    v.extend_from_slice(pending.as_ref());
    v.extend_from_slice(&pr.unit_amount.to_le_bytes());
    v.extend_from_slice(&(pr.balls as u32).to_le_bytes());
    v.extend_from_slice(&[pr.rows]);
    v.extend_from_slice(&[pr.difficulty]);
    v.extend_from_slice(&payout.to_le_bytes()); // NET payout
    v.extend_from_slice(&pr.nonce.to_le_bytes());
    v.extend_from_slice(&pr.expiry_unix.to_le_bytes());
    v
}
