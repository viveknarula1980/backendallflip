use anchor_lang::system_program::ID as OtherID;
use anchor_lang::prelude::*;
use anchor_lang::system_program::System;
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

declare_id!("2XSiZfPQDAHv6XWTiDxunPnDaqoMDNAxn3FmY4dYQAeT"); // ← REPLACE after deploy

// ---- constants ----
const MAX_PAYOUT_LAMPORTS: u64 = 50_000_000_000; // 0.05 SOL (tune)
const MIN_BET_LAMPORTS: u64  = 50_000;           // 0.00005 SOL
const MAX_BET_LAMPORTS: u64  = 5_000_000_000;    // 5 SOL
const FEE_REIMBURSE_LAMPORTS: u64 = 1_400_000;   // user_vault → server fee payer (set 0 to disable)

#[error_code]
pub enum CasinoErr {
    #[msg("Invalid ed25519 pre-instruction")] InvalidEd25519,
    #[msg("Expired signature/nonce")]        Expired,
    #[msg("Bad params")]                     BadParams,
    #[msg("Vault mismatch")]                 VaultMismatch,
    #[msg("Insufficient vault balance")]     InsufficientVault,
    #[msg("Payout sanity check failed")]     BadPayout,
    #[msg("Already settled or not found")]   BadPending,
}

// ---- accounts ----
#[account]
pub struct AdminConfig { pub admin_pubkey: [u8; 32] }

#[account]
pub struct UserVault {
    pub owner: Pubkey,
    pub bump:  u8,
    pub _r1:   [u8; 7],
    pub _r2:   [u8; 32],
    pub _r3:   i64,
    pub _r4:   u64,
    pub _r5:   u64,
}
impl UserVault { pub const LEN: usize = 8 + 32 + 1 + 7 + 32 + 8 + 8 + 8; }

// dice pending
#[account]
pub struct PendingBet {
    pub player: Pubkey,
    pub amount: u64,
    pub bet_type: u8, // 0 under, 1 over
    pub target:  u8,  // 2..98
    pub nonce:   u64,
    pub expiry_unix: i64,
    pub settled: bool,
}
impl PendingBet { pub const LEN: usize = 8 + 32 + 8 + 1 + 1 + 8 + 8 + 1; }

// mines pending
#[account]
pub struct PendingRound {
    pub player: Pubkey,
    pub amount: u64,
    pub rows: u8,
    pub cols: u8,
    pub mines: u8,
    pub nonce: u64,
    pub expiry_unix: i64,
    pub settled: bool,
}
impl PendingRound { pub const LEN: usize = 8 + 32 + 8 + 1 + 1 + 1 + 8 + 8 + 1; }

// ---- events ----
#[event] pub struct DiceLocked   { pub player: Pubkey, pub amount: u64, pub bet_type: u8, pub target: u8, pub nonce: u64 }
#[event] pub struct DiceResolved { pub player: Pubkey, pub win: bool,   pub roll: u8,    pub payout: u64, pub nonce: u64 }

#[event] pub struct MinesLocked   { pub player: Pubkey, pub amount: u64, pub rows: u8, pub cols: u8, pub mines: u8, pub nonce: u64 }
#[event] pub struct MinesResolved { pub player: Pubkey, pub payout: u64, pub checksum: u8, pub nonce: u64 }

// ---- utils ----
fn safe_move_lamports(from: &AccountInfo<'_>, to: &AccountInfo<'_>, amount: u64) -> Result<()> {
    require!(amount > 0, CasinoErr::BadParams);
    let mut from_lamports = from.try_borrow_mut_lamports()?;
    let mut to_lamports   = to.try_borrow_mut_lamports()?;
    require!(**from_lamports >= amount, CasinoErr::InsufficientVault);
    **from_lamports -= amount;
    **to_lamports   += amount;
    Ok(())
}

fn require_ed25519_present(sys_ix_ai: &AccountInfo<'_>, hinted_idx: u8) -> Result<()> {
    let hinted_ok = load_instruction_at_checked(hinted_idx as usize, sys_ix_ai)
        .map(|ix| ix.program_id == ed25519_program::id())
        .unwrap_or(false);
    if hinted_ok { return Ok(()); }
    let cur_idx = load_current_index_checked(sys_ix_ai)?;
    for i in 0..cur_idx {
        if let Ok(ix) = load_instruction_at_checked(i as usize, sys_ix_ai) {
            if ix.program_id == ed25519_program::id() { return Ok(()); }
        }
    }
    err!(CasinoErr::InvalidEd25519)
}

// ---- contexts ----
#[derive(Accounts)]
pub struct InitAdmin<'info> {
    #[account(mut, signer)] pub authority: SystemAccount<'info>,
    #[account(init, payer=authority, space=8+32, seeds=[b"admin"], bump)]
    pub admin_config: Account<'info, AdminConfig>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct InitHouseVault<'info> {
    #[account(mut, signer)] pub payer: SystemAccount<'info>,
    #[account(mut, seeds=[b"vault"], bump)]
    /// CHECK: system-owned lamports PDA
    pub house_vault: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ActivateUserVault<'info> {
    #[account(mut, signer)] pub player: SystemAccount<'info>,
    #[account(init, payer=player, space=UserVault::LEN, seeds=[b"user_vault", player.key().as_ref()], bump)]
    pub user_vault: Account<'info, UserVault>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct DepositToVault<'info> {
    #[account(mut, signer)] pub player: SystemAccount<'info>,
    #[account(mut, seeds=[b"user_vault", player.key().as_ref()], bump=user_vault.bump)]
    pub user_vault: Account<'info, UserVault>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawFromVault<'info> {
    #[account(mut, signer)] pub player: SystemAccount<'info>,
    #[account(mut, seeds=[b"user_vault", player.key().as_ref()], bump=user_vault.bump)]
    pub user_vault: Account<'info, UserVault>,
    pub system_program: Program<'info, System>,
}

// dice
#[derive(Accounts)]
#[instruction(args: DiceLockArgs)]
pub struct DiceLock<'info> {
    pub player: SystemAccount<'info>,
    #[account(mut, signer)] pub fee_payer: SystemAccount<'info>,
    #[account(mut, seeds=[b"user_vault", player.key().as_ref()], bump=user_vault.bump)]
    pub user_vault: Account<'info, UserVault>,
    #[account(mut, seeds=[b"vault"], bump)]
    pub house_vault: SystemAccount<'info>,
    #[account(init, payer=fee_payer, space=PendingBet::LEN, seeds=[b"bet", player.key().as_ref(), &args.nonce.to_le_bytes()], bump)]
    pub pending_bet: Account<'info, PendingBet>,
    pub system_program: Program<'info, System>,
    /// CHECK
    #[account(address = SYSVAR_INSTRUCTIONS_ID)]
    pub sysvar_instructions: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct DiceResolve<'info> {
    #[account(mut)] pub player: SystemAccount<'info>,
    #[account(mut, seeds=[b"vault"], bump)] pub house_vault: SystemAccount<'info>,
    #[account(seeds=[b"admin"], bump)] pub admin_config: Account<'info, AdminConfig>,
    #[account(mut, seeds=[b"user_vault", player.key().as_ref()], bump=user_vault.bump)]
    pub user_vault: Account<'info, UserVault>,
    #[account(mut, close=user_vault, seeds=[b"bet", player.key().as_ref(), &pending_bet.nonce.to_le_bytes()], bump)]
    pub pending_bet: Account<'info, PendingBet>,
    pub system_program: Program<'info, System>,
    /// CHECK
    #[account(address = SYSVAR_INSTRUCTIONS_ID)]
    pub sysvar_instructions: UncheckedAccount<'info>,
}

// mines
#[derive(Accounts)]
#[instruction(args: MinesLockArgs)]
pub struct MinesLock<'info> {
    pub player: SystemAccount<'info>,
    #[account(mut, signer)] pub fee_payer: SystemAccount<'info>,
    #[account(mut, seeds=[b"user_vault", player.key().as_ref()], bump=user_vault.bump)]
    pub user_vault: Account<'info, UserVault>,
    #[account(mut, seeds=[b"vault"], bump)]
    pub house_vault: SystemAccount<'info>,
    #[account(init, payer=fee_payer, space=PendingRound::LEN, seeds=[b"round", player.key().as_ref(), &args.nonce.to_le_bytes()], bump)]
    pub pending: Account<'info, PendingRound>,
    pub system_program: Program<'info, System>,
    /// CHECK
    #[account(address = SYSVAR_INSTRUCTIONS_ID)]
    pub sysvar_instructions: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct MinesResolve<'info> {
    #[account(mut)] pub player: SystemAccount<'info>,
    #[account(mut, seeds=[b"vault"], bump)] pub house_vault: SystemAccount<'info>,
    #[account(seeds=[b"admin"], bump)] pub admin_config: Account<'info, AdminConfig>,
    #[account(mut, seeds=[b"user_vault", player.key().as_ref()], bump=user_vault.bump)]
    pub user_vault: Account<'info, UserVault>,
    #[account(mut, close=user_vault, seeds=[b"round", player.key().as_ref(), &pending.nonce.to_le_bytes()], bump)]
    pub pending: Account<'info, PendingRound>,
    pub system_program: Program<'info, System>,
    /// CHECK
    #[account(address = SYSVAR_INSTRUCTIONS_ID)]
    pub sysvar_instructions: UncheckedAccount<'info>,
}

// ---- args ----
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct ActivateArgs { pub initial_deposit: u64 }
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct DepositArgs  { pub amount: u64 }
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct WithdrawArgs { pub amount: u64 }

// dice
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct DiceLockArgs {
    pub bet_amount: u64,
    pub bet_type: u8,      // 0 under, 1 over
    pub target:  u8,       // 2..98
    pub nonce: u64,
    pub expiry_unix: i64,
    pub ed25519_instr_index: u8,
}
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct DiceResolveArgs {
    pub roll: u8,
    pub payout: u64,
    pub ed25519_instr_index: u8,
}

// mines
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct MinesLockArgs {
    pub bet_amount: u64,
    pub rows: u8, pub cols: u8, pub mines: u8,
    pub nonce: u64,
    pub expiry_unix: i64,
    pub ed25519_instr_index: u8,
}
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct MinesResolveArgs {
    pub checksum: u8,
    pub payout: u64,
    pub ed25519_instr_index: u8,
}

// ---- program ----
#[program]
pub mod casino {
    use super::*;

    pub fn init_admin(ctx: Context<InitAdmin>, admin_pubkey: [u8; 32]) -> Result<()> {
        ctx.accounts.admin_config.admin_pubkey = admin_pubkey; Ok(())
    }

    pub fn init_house_vault(ctx: Context<InitHouseVault>) -> Result<()> {
        let rent = Rent::get()?.minimum_balance(0);
        let bump = ctx.bumps.house_vault;
        let ix = system_instruction::create_account(
            &ctx.accounts.payer.key(),
            &ctx.accounts.house_vault.key(),
            rent.max(1),
            0,
            &System::id(),
        );
        invoke_signed(
            &ix,
            &[
                ctx.accounts.payer.to_account_info(),
                ctx.accounts.house_vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[&[b"vault", &[bump]]],
        )?;
        Ok(())
    }

    pub fn activate_user_vault(ctx: Context<ActivateUserVault>, args: ActivateArgs) -> Result<()> {
        let uv = &mut ctx.accounts.user_vault;
        uv.owner = ctx.accounts.player.key();
        uv.bump  = ctx.bumps.user_vault;
        if args.initial_deposit > 0 {
            let ix = system_instruction::transfer(&ctx.accounts.player.key(), &ctx.accounts.user_vault.key(), args.initial_deposit);
            invoke(&ix, &[
                ctx.accounts.player.to_account_info(),
                ctx.accounts.user_vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ])?;
        }
        Ok(())
    }

    pub fn deposit_to_vault(ctx: Context<DepositToVault>, args: DepositArgs) -> Result<()> {
        require!(args.amount > 0, CasinoErr::BadParams);
        let ix = system_instruction::transfer(&ctx.accounts.player.key(), &ctx.accounts.user_vault.key(), args.amount);
        invoke(&ix, &[
            ctx.accounts.player.to_account_info(),
            ctx.accounts.user_vault.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ])?;
        Ok(())
    }

    pub fn withdraw_from_vault(ctx: Context<WithdrawFromVault>, args: WithdrawArgs) -> Result<()> {
        require!(args.amount > 0, CasinoErr::BadParams);
        let from = ctx.accounts.user_vault.to_account_info();
        let to   = ctx.accounts.player.to_account_info();
        safe_move_lamports(&from, &to, args.amount)
    }

    // ---- dice ----
    pub fn dice_lock(ctx: Context<DiceLock>, args: DiceLockArgs) -> Result<()> {
        require!(args.bet_amount >= MIN_BET_LAMPORTS && args.bet_amount <= MAX_BET_LAMPORTS, CasinoErr::BadParams);
        require!(args.target >= 2 && args.target <= 98, CasinoErr::BadParams);
        require!(args.bet_type <= 1, CasinoErr::BadParams);

        require_ed25519_present(&ctx.accounts.sysvar_instructions.to_account_info(), args.ed25519_instr_index)?;
        require!(ctx.accounts.user_vault.owner == ctx.accounts.player.key(), CasinoErr::VaultMismatch);

        let uv_bal = **ctx.accounts.user_vault.to_account_info().lamports.borrow();
        let need = args.bet_amount.saturating_add(FEE_REIMBURSE_LAMPORTS);
        require!(uv_bal >= need, CasinoErr::InsufficientVault);

        let uv_ai = ctx.accounts.user_vault.to_account_info();
        let hv_ai = ctx.accounts.house_vault.to_account_info();
        safe_move_lamports(&uv_ai, &hv_ai, args.bet_amount)?;

        if FEE_REIMBURSE_LAMPORTS > 0 {
            let fp_ai = ctx.accounts.fee_payer.to_account_info();
            safe_move_lamports(&uv_ai, &fp_ai, FEE_REIMBURSE_LAMPORTS)?;
        }

        let pb = &mut ctx.accounts.pending_bet;
        pb.player = ctx.accounts.player.key();
        pb.amount = args.bet_amount;
        pb.bet_type = args.bet_type;
        pb.target = args.target;
        pb.nonce = args.nonce;
        pb.expiry_unix = args.expiry_unix;
        pb.settled = false;

        emit!(DiceLocked { player: pb.player, amount: pb.amount, bet_type: pb.bet_type, target: pb.target, nonce: pb.nonce });
        Ok(())
    }

    pub fn dice_resolve(ctx: Context<DiceResolve>, args: DiceResolveArgs) -> Result<()> {
        let pb = &mut ctx.accounts.pending_bet;
        require!(!pb.settled, CasinoErr::BadPending);

        let clock = Clock::get()?;
        require!(clock.unix_timestamp <= pb.expiry_unix, CasinoErr::Expired);
        require_ed25519_present(&ctx.accounts.sysvar_instructions.to_account_info(), args.ed25519_instr_index)?;

        require!(args.roll >= 1 && args.roll <= 100, CasinoErr::BadParams);
        let win = match pb.bet_type { 0 => args.roll < pb.target, _ => args.roll > pb.target };
        if win {
            require!(args.payout > 0 && args.payout <= MAX_PAYOUT_LAMPORTS, CasinoErr::BadPayout);
        } else {
            require!(args.payout == 0, CasinoErr::BadPayout);
        }

        if win && args.payout > 0 {
            let bump_v = ctx.bumps.house_vault;
            let ix = system_instruction::transfer(&ctx.accounts.house_vault.key(), &ctx.accounts.user_vault.key(), args.payout);
            invoke_signed(&ix, &[
                ctx.accounts.house_vault.to_account_info(),
                ctx.accounts.user_vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ], &[&[b"vault", &[bump_v]]])?;
        }

        pb.settled = true;
        emit!(DiceResolved { player: pb.player, win, roll: args.roll, payout: args.payout, nonce: pb.nonce });
        Ok(())
    }

    // ---- mines ----
    pub fn mines_lock(ctx: Context<MinesLock>, args: MinesLockArgs) -> Result<()> {
        require!(args.bet_amount >= MIN_BET_LAMPORTS && args.bet_amount <= MAX_BET_LAMPORTS, CasinoErr::BadParams);
        require!(args.rows >= 2 && args.rows <= 8, CasinoErr::BadParams);
        require!(args.cols >= 2 && args.cols <= 8, CasinoErr::BadParams);
        let total = (args.rows as u16) * (args.cols as u16);
        require!(args.mines >= 1 && (args.mines as u16) < total, CasinoErr::BadParams);

        require_ed25519_present(&ctx.accounts.sysvar_instructions.to_account_info(), args.ed25519_instr_index)?;
        require!(ctx.accounts.user_vault.owner == ctx.accounts.player.key(), CasinoErr::VaultMismatch);

        let uv_bal = **ctx.accounts.user_vault.to_account_info().lamports.borrow();
        let need = args.bet_amount.saturating_add(FEE_REIMBURSE_LAMPORTS);
        require!(uv_bal >= need, CasinoErr::InsufficientVault);

        let uv_ai = ctx.accounts.user_vault.to_account_info();
        let hv_ai = ctx.accounts.house_vault.to_account_info();
        safe_move_lamports(&uv_ai, &hv_ai, args.bet_amount)?;

        if FEE_REIMBURSE_LAMPORTS > 0 {
            let fp_ai = ctx.accounts.fee_payer.to_account_info();
            safe_move_lamports(&uv_ai, &fp_ai, FEE_REIMBURSE_LAMPORTS)?;
        }

        let p = &mut ctx.accounts.pending;
        p.player = ctx.accounts.player.key();
        p.amount = args.bet_amount;
        p.rows = args.rows;
        p.cols = args.cols;
        p.mines = args.mines;
        p.nonce = args.nonce;
        p.expiry_unix = args.expiry_unix;
        p.settled = false;

        emit!(MinesLocked { player: p.player, amount: p.amount, rows: p.rows, cols: p.cols, mines: p.mines, nonce: p.nonce });
        Ok(())
    }

    pub fn mines_resolve(ctx: Context<MinesResolve>, args: MinesResolveArgs) -> Result<()> {
        let p = &mut ctx.accounts.pending;
        require!(!p.settled, CasinoErr::BadPending);

        let clock = Clock::get()?;
        require!(clock.unix_timestamp <= p.expiry_unix, CasinoErr::Expired);
        require_ed25519_present(&ctx.accounts.sysvar_instructions.to_account_info(), args.ed25519_instr_index)?;

        let expected = ((p.nonce % 251) + 1) as u8;
        require!(args.checksum == expected, CasinoErr::BadParams);
        require!(args.payout <= MAX_PAYOUT_LAMPORTS, CasinoErr::BadPayout);

        if args.payout > 0 {
            let bump_v = ctx.bumps.house_vault;
            let ix = system_instruction::transfer(&ctx.accounts.house_vault.key(), &ctx.accounts.user_vault.key(), args.payout);
            invoke_signed(&ix, &[
                ctx.accounts.house_vault.to_account_info(),
                ctx.accounts.user_vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ], &[&[b"vault", &[bump_v]]])?;
        }

        p.settled = true;
        emit!(MinesResolved { player: p.player, payout: args.payout, checksum: args.checksum, nonce: p.nonce });
        Ok(())
    }
}
