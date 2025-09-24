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

declare_id!("5vgLU8GyehUkziMaKHCtyPu6YZgo11wct8rTHLdz4z1"); // ← REPLACE after deploy

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

// ---- pending accounts per game ----
#[account]
pub struct PendingDice {
    pub player: Pubkey,
    pub amount: u64,
    pub bet_type: u8, // 0 under, 1 over
    pub target:  u8,  // 2..98
    pub nonce:   u64,
    pub expiry_unix: i64,
    pub settled: bool,
}
impl PendingDice { pub const LEN: usize = 8 + 32 + 8 + 1 + 1 + 8 + 8 + 1; }

#[account]
pub struct PendingMines {
    pub player: Pubkey,
    pub amount: u64,
    pub rows: u8,
    pub cols: u8,
    pub mines: u8,
    pub nonce: u64,
    pub expiry_unix: i64,
    pub settled: bool,
}
impl PendingMines { pub const LEN: usize = 8 + 32 + 8 + 1 + 1 + 1 + 8 + 8 + 1; }

#[account]
pub struct PendingFlip {
    pub player: Pubkey,
    pub amount: u64,
    pub side: u8,        // 0=heads,1=tails (player pick)
    pub nonce: u64,
    pub expiry_unix: i64,
    pub settled: bool,
}
impl PendingFlip { pub const LEN: usize = 8 + 32 + 8 + 1 + 8 + 8 + 1; }

#[account]
pub struct PendingCrash {
    pub player: Pubkey,
    pub amount: u64,
    pub nonce: u64,
    pub expiry_unix: i64,
    pub settled: bool,
}
impl PendingCrash { pub const LEN: usize = 8 + 32 + 8 + 8 + 8 + 1; }

#[account]
pub struct PendingPlinko {
    pub player: Pubkey,
    pub unit_amount: u64, // per ball
    pub balls: u16,
    pub rows: u8,
    pub difficulty: u8,   // 0 easy, 1 med, 2 hard
    pub nonce: u64,
    pub expiry_unix: i64,
    pub settled: bool,
}
impl PendingPlinko { pub const LEN: usize = 8 + 32 + 8 + 2 + 1 + 1 + 8 + 8 + 1; }

#[account]
pub struct PendingSlots {
    pub player: Pubkey,
    pub amount: u64,
    pub nonce: u64,
    pub expiry_unix: i64,
    pub settled: bool,
}
impl PendingSlots { pub const LEN: usize = 8 + 32 + 8 + 8 + 8 + 1; }

// ---- events ----
#[event] pub struct DiceLocked   { pub player: Pubkey, pub amount: u64, pub bet_type: u8, pub target: u8, pub nonce: u64 }
#[event] pub struct DiceResolved { pub player: Pubkey, pub win: bool,   pub roll: u8,    pub payout: u64, pub nonce: u64 }

#[event] pub struct MinesLocked   { pub player: Pubkey, pub amount: u64, pub rows: u8, pub cols: u8, pub mines: u8, pub nonce: u64 }
#[event] pub struct MinesResolved { pub player: Pubkey, pub payout: u64, pub checksum: u8, pub nonce: u64 }

#[event] pub struct FlipLocked   { pub player: Pubkey, pub amount: u64, pub side: u8, pub nonce: u64 }
#[event] pub struct FlipResolved { pub player: Pubkey, pub winner_side: u8, pub payout: u64, pub nonce: u64 }

#[event] pub struct CrashLocked   { pub player: Pubkey, pub amount: u64, pub nonce: u64 }
#[event] pub struct CrashResolved { pub player: Pubkey, pub multiplier_bps: u32, pub payout: u64, pub nonce: u64 }

#[event] pub struct PlinkoLocked   { pub player: Pubkey, pub unit_amount: u64, pub balls: u16, pub rows: u8, pub difficulty: u8, pub nonce: u64 }
#[event] pub struct PlinkoResolved { pub player: Pubkey, pub total_payout: u64, pub checksum: u8, pub nonce: u64 }

#[event] pub struct SlotsLocked    { pub player: Pubkey, pub amount: u64, pub nonce: u64 }
#[event] pub struct SlotsResolved  { pub player: Pubkey, pub payout: u64, pub checksum: u8, pub nonce: u64 }

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

// ---- contexts (shared) ----
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

// flip (coinflip)
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct FlipLockArgs {
    pub bet_amount: u64,
    pub side: u8,
    pub nonce: u64,
    pub expiry_unix: i64,
    pub ed25519_instr_index: u8,
}
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct FlipResolveArgs {
    pub winner_side: u8,
    pub payout: u64,
    pub ed25519_instr_index: u8,
}

// crash
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct CrashLockArgs {
    pub bet_amount: u64,
    pub nonce: u64,
    pub expiry_unix: i64,
    pub ed25519_instr_index: u8,
}
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct CrashResolveArgs {
    pub multiplier_bps: u32,
    pub payout: u64,
    pub ed25519_instr_index: u8,
}

// plinko
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct PlinkoLockArgs {
    pub unit_amount: u64, pub balls: u16, pub rows: u8, pub difficulty: u8,
    pub nonce: u64,
    pub expiry_unix: i64,
    pub ed25519_instr_index: u8,
}
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct PlinkoResolveArgs {
    pub checksum: u8,
    pub total_payout: u64,
    pub ed25519_instr_index: u8,
}

// slots
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct SlotsLockArgs {
    pub bet_amount: u64,
    pub nonce: u64,
    pub expiry_unix: i64,
    pub ed25519_instr_index: u8,
}
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct SlotsResolveArgs {
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
    #[derive(Accounts)]
    #[instruction(args: DiceLockArgs)]
    pub struct DiceLock<'info> {
        pub player: SystemAccount<'info>,
        #[account(mut, signer)] pub fee_payer: SystemAccount<'info>,
        #[account(mut, seeds=[b"user_vault", player.key().as_ref()], bump=user_vault.bump)]
        pub user_vault: Account<'info, UserVault>,
        #[account(mut, seeds=[b"vault"], bump)]
        pub house_vault: SystemAccount<'info>,
        #[account(init, payer=fee_payer, space=PendingDice::LEN, seeds=[b"bet", player.key().as_ref(), &args.nonce.to_le_bytes()], bump)]
        pub pending: Account<'info, PendingDice>,
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
        #[account(mut, close=user_vault, seeds=[b"bet", player.key().as_ref(), &pending.nonce.to_le_bytes()], bump)]
        pub pending: Account<'info, PendingDice>,
        pub system_program: Program<'info, System>,
        /// CHECK
        #[account(address = SYSVAR_INSTRUCTIONS_ID)]
        pub sysvar_instructions: UncheckedAccount<'info>,
    }

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

        let p = &mut ctx.accounts.pending;
        p.player = ctx.accounts.player.key();
        p.amount = args.bet_amount;
        p.bet_type = args.bet_type;
        p.target = args.target;
        p.nonce = args.nonce;
        p.expiry_unix = args.expiry_unix;
        p.settled = false;

        emit!(DiceLocked { player: p.player, amount: p.amount, bet_type: p.bet_type, target: p.target, nonce: p.nonce });
        Ok(())
    }

    pub fn dice_resolve(ctx: Context<DiceResolve>, args: DiceResolveArgs) -> Result<()> {
        let p = &mut ctx.accounts.pending;
        require!(!p.settled, CasinoErr::BadPending);

        let clock = Clock::get()?;
        require!(clock.unix_timestamp <= p.expiry_unix, CasinoErr::Expired);
        require_ed25519_present(&ctx.accounts.sysvar_instructions.to_account_info(), args.ed25519_instr_index)?;

        require!(args.roll >= 1 && args.roll <= 100, CasinoErr::BadParams);
        let win = match p.bet_type { 0 => args.roll < p.target, _ => args.roll > p.target };
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

        p.settled = true;
        emit!(DiceResolved { player: p.player, win, roll: args.roll, payout: args.payout, nonce: p.nonce });
        Ok(())
    }

    // ---- mines ----
    #[derive(Accounts)]
    #[instruction(args: MinesLockArgs)]
    pub struct MinesLock<'info> {
        pub player: SystemAccount<'info>,
        #[account(mut, signer)] pub fee_payer: SystemAccount<'info>,
        #[account(mut, seeds=[b"user_vault", player.key().as_ref()], bump=user_vault.bump)]
        pub user_vault: Account<'info, UserVault>,
        #[account(mut, seeds=[b"vault"], bump)]
        pub house_vault: SystemAccount<'info>,
        #[account(init, payer=fee_payer, space=PendingMines::LEN, seeds=[b"round", player.key().as_ref(), &args.nonce.to_le_bytes()], bump)]
        pub pending: Account<'info, PendingMines>,
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
        pub pending: Account<'info, PendingMines>,
        pub system_program: Program<'info, System>,
        /// CHECK
        #[account(address = SYSVAR_INSTRUCTIONS_ID)]
        pub sysvar_instructions: UncheckedAccount<'info>,
    }

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

    // ---- coinflip ----
    #[derive(Accounts)]
    #[instruction(args: FlipLockArgs)]
    pub struct FlipLock<'info> {
        pub player: SystemAccount<'info>,
        #[account(mut, signer)] pub fee_payer: SystemAccount<'info>,
        #[account(mut, seeds=[b"user_vault", player.key().as_ref()], bump=user_vault.bump)]
        pub user_vault: Account<'info, UserVault>,
        #[account(mut, seeds=[b"vault"], bump)]
        pub house_vault: SystemAccount<'info>,
        #[account(init, payer=fee_payer, space=PendingFlip::LEN, seeds=[b"flip", player.key().as_ref(), &args.nonce.to_le_bytes()], bump)]
        pub pending: Account<'info, PendingFlip>,
        pub system_program: Program<'info, System>,
        /// CHECK
        #[account(address = SYSVAR_INSTRUCTIONS_ID)]
        pub sysvar_instructions: UncheckedAccount<'info>,
    }
    #[derive(Accounts)]
    pub struct FlipResolve<'info> {
        #[account(mut)] pub player: SystemAccount<'info>,
        #[account(mut, seeds=[b"vault"], bump)] pub house_vault: SystemAccount<'info>,
        #[account(seeds=[b"admin"], bump)] pub admin_config: Account<'info, AdminConfig>,
        #[account(mut, seeds=[b"user_vault", player.key().as_ref()], bump=user_vault.bump)]
        pub user_vault: Account<'info, UserVault>,
        #[account(mut, close=user_vault, seeds=[b"flip", player.key().as_ref(), &pending.nonce.to_le_bytes()], bump)]
        pub pending: Account<'info, PendingFlip>,
        pub system_program: Program<'info, System>,
        /// CHECK
        #[account(address = SYSVAR_INSTRUCTIONS_ID)]
        pub sysvar_instructions: UncheckedAccount<'info>,
    }

    pub fn flip_lock(ctx: Context<FlipLock>, args: FlipLockArgs) -> Result<()> {
        require!(args.bet_amount >= MIN_BET_LAMPORTS && args.bet_amount <= MAX_BET_LAMPORTS, CasinoErr::BadParams);
        require!(args.side <= 1, CasinoErr::BadParams);
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
        p.player = ctx.accounts.player.key(); p.amount = args.bet_amount; p.side = args.side;
        p.nonce = args.nonce; p.expiry_unix = args.expiry_unix; p.settled = false;
        emit!(FlipLocked { player: p.player, amount: p.amount, side: p.side, nonce: p.nonce });
        Ok(())
    }

    pub fn flip_resolve(ctx: Context<FlipResolve>, args: FlipResolveArgs) -> Result<()> {
        let p = &mut ctx.accounts.pending;
        require!(!p.settled, CasinoErr::BadPending);

        let clock = Clock::get()?;
        require!(clock.unix_timestamp <= p.expiry_unix, CasinoErr::Expired);
        require_ed25519_present(&ctx.accounts.sysvar_instructions.to_account_info(), args.ed25519_instr_index)?;

        require!(args.winner_side <= 1, CasinoErr::BadParams);
        let win = args.winner_side == p.side;
        if win { require!(args.payout > 0 && args.payout <= MAX_PAYOUT_LAMPORTS, CasinoErr::BadPayout); }
        else   { require!(args.payout == 0, CasinoErr::BadPayout); }

        if win && args.payout > 0 {
            let bump_v = ctx.bumps.house_vault;
            let ix = system_instruction::transfer(&ctx.accounts.house_vault.key(), &ctx.accounts.user_vault.key(), args.payout);
            invoke_signed(&ix, &[
                ctx.accounts.house_vault.to_account_info(),
                ctx.accounts.user_vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ], &[&[b"vault", &[bump_v]]])?;
        }
        p.settled = true;
        emit!(FlipResolved { player: p.player, winner_side: args.winner_side, payout: args.payout, nonce: p.nonce });
        Ok(())
    }

    // ---- crash ----
    #[derive(Accounts)]
    #[instruction(args: CrashLockArgs)]
    pub struct CrashLock<'info> {
        pub player: SystemAccount<'info>,
        #[account(mut, signer)] pub fee_payer: SystemAccount<'info>,
        #[account(mut, seeds=[b"user_vault", player.key().as_ref()], bump=user_vault.bump)]
        pub user_vault: Account<'info, UserVault>,
        #[account(mut, seeds=[b"vault"], bump)]
        pub house_vault: SystemAccount<'info>,
        #[account(init, payer=fee_payer, space=PendingCrash::LEN, seeds=[b"crash", player.key().as_ref(), &args.nonce.to_le_bytes()], bump)]
        pub pending: Account<'info, PendingCrash>,
        pub system_program: Program<'info, System>,
        /// CHECK
        #[account(address = SYSVAR_INSTRUCTIONS_ID)]
        pub sysvar_instructions: UncheckedAccount<'info>,
    }
    #[derive(Accounts)]
    pub struct CrashResolve<'info> {
        #[account(mut)] pub player: SystemAccount<'info>,
        #[account(mut, seeds=[b"vault"], bump)] pub house_vault: SystemAccount<'info>,
        #[account(seeds=[b"admin"], bump)] pub admin_config: Account<'info, AdminConfig>,
        #[account(mut, seeds=[b"user_vault", player.key().as_ref()], bump=user_vault.bump)]
        pub user_vault: Account<'info, UserVault>,
        #[account(mut, close=user_vault, seeds=[b"crash", player.key().as_ref(), &pending.nonce.to_le_bytes()], bump)]
        pub pending: Account<'info, PendingCrash>,
        pub system_program: Program<'info, System>,
        /// CHECK
        #[account(address = SYSVAR_INSTRUCTIONS_ID)]
        pub sysvar_instructions: UncheckedAccount<'info>,
    }

    pub fn crash_lock(ctx: Context<CrashLock>, args: CrashLockArgs) -> Result<()> {
        require!(args.bet_amount >= MIN_BET_LAMPORTS && args.bet_amount <= MAX_BET_LAMPORTS, CasinoErr::BadParams);
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
        p.player = ctx.accounts.player.key(); p.amount = args.bet_amount; p.nonce = args.nonce; p.expiry_unix = args.expiry_unix; p.settled = false;
        emit!(CrashLocked { player: p.player, amount: p.amount, nonce: p.nonce });
        Ok(())
    }

    pub fn crash_resolve(ctx: Context<CrashResolve>, args: CrashResolveArgs) -> Result<()> {
        let p = &mut ctx.accounts.pending;
        require!(!p.settled, CasinoErr::BadPending);

        let clock = Clock::get()?;
        require!(clock.unix_timestamp <= p.expiry_unix, CasinoErr::Expired);
        require_ed25519_present(&ctx.accounts.sysvar_instructions.to_account_info(), args.ed25519_instr_index)?;

        require!(args.multiplier_bps >= 10000, CasinoErr::BadParams); // >=1x
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
        emit!(CrashResolved { player: p.player, multiplier_bps: args.multiplier_bps, payout: args.payout, nonce: p.nonce });
        Ok(())
    }

    // ---- plinko ----
    #[derive(Accounts)]
    #[instruction(args: PlinkoLockArgs)]
    pub struct PlinkoLock<'info> {
        pub player: SystemAccount<'info>,
        #[account(mut, signer)] pub fee_payer: SystemAccount<'info>,
        #[account(mut, seeds=[b"user_vault", player.key().as_ref()], bump=user_vault.bump)]
        pub user_vault: Account<'info, UserVault>,
        #[account(mut, seeds=[b"vault"], bump)] pub house_vault: SystemAccount<'info>,
        #[account(init, payer=fee_payer, space=PendingPlinko::LEN, seeds=[b"plinkobet", player.key().as_ref(), &args.nonce.to_le_bytes()], bump)]
        pub pending: Account<'info, PendingPlinko>,
        pub system_program: Program<'info, System>,
        /// CHECK
        #[account(address = SYSVAR_INSTRUCTIONS_ID)]
        pub sysvar_instructions: UncheckedAccount<'info>,
    }
    #[derive(Accounts)]
    pub struct PlinkoResolve<'info> {
        #[account(mut)] pub player: SystemAccount<'info>,
        #[account(mut, seeds=[b"vault"], bump)] pub house_vault: SystemAccount<'info>,
        #[account(seeds=[b"admin"], bump)] pub admin_config: Account<'info, AdminConfig>,
        #[account(mut, seeds=[b"user_vault", player.key().as_ref()], bump=user_vault.bump)]
        pub user_vault: Account<'info, UserVault>,
        #[account(mut, close=user_vault, seeds=[b"plinkobet", player.key().as_ref(), &pending.nonce.to_le_bytes()], bump)]
        pub pending: Account<'info, PendingPlinko>,
        pub system_program: Program<'info, System>,
        /// CHECK
        #[account(address = SYSVAR_INSTRUCTIONS_ID)]
        pub sysvar_instructions: UncheckedAccount<'info>,
    }

    pub fn plinko_lock(ctx: Context<PlinkoLock>, args: PlinkoLockArgs) -> Result<()> {
        require!(args.unit_amount >= MIN_BET_LAMPORTS && args.unit_amount <= MAX_BET_LAMPORTS, CasinoErr::BadParams);
        require!(args.balls >= 1, CasinoErr::BadParams);
        require!(args.rows >= 8 && args.rows <= 16, CasinoErr::BadParams);
        require!(args.difficulty <= 2, CasinoErr::BadParams); // 0,1,2

        require_ed25519_present(&ctx.accounts.sysvar_instructions.to_account_info(), args.ed25519_instr_index)?;
        require!(ctx.accounts.user_vault.owner == ctx.accounts.player.key(), CasinoErr::VaultMismatch);

        let total = (args.unit_amount as u128) * (args.balls as u128);
        require!(total <= (MAX_BET_LAMPORTS as u128), CasinoErr::BadParams);

        let uv_bal = **ctx.accounts.user_vault.to_account_info().lamports.borrow();
        let need = (total as u64).saturating_add(FEE_REIMBURSE_LAMPORTS);
        require!(uv_bal >= need, CasinoErr::InsufficientVault);

        let uv_ai = ctx.accounts.user_vault.to_account_info();
        let hv_ai = ctx.accounts.house_vault.to_account_info();
        safe_move_lamports(&uv_ai, &hv_ai, total as u64)?;

        if FEE_REIMBURSE_LAMPORTS > 0 {
            let fp_ai = ctx.accounts.fee_payer.to_account_info();
            safe_move_lamports(&uv_ai, &fp_ai, FEE_REIMBURSE_LAMPORTS)?;
        }

        let p = &mut ctx.accounts.pending;
        p.player = ctx.accounts.player.key(); p.unit_amount = args.unit_amount; p.balls = args.balls; p.rows = args.rows; p.difficulty = args.difficulty;
        p.nonce = args.nonce; p.expiry_unix = args.expiry_unix; p.settled = false;
        emit!(PlinkoLocked { player: p.player, unit_amount: p.unit_amount, balls: p.balls, rows: p.rows, difficulty: p.difficulty, nonce: p.nonce });
        Ok(())
    }

    pub fn plinko_resolve(ctx: Context<PlinkoResolve>, args: PlinkoResolveArgs) -> Result<()> {
        let p = &mut ctx.accounts.pending;
        require!(!p.settled, CasinoErr::BadPending);

        let clock = Clock::get()?;
        require!(clock.unix_timestamp <= p.expiry_unix, CasinoErr::Expired);
        require_ed25519_present(&ctx.accounts.sysvar_instructions.to_account_info(), args.ed25519_instr_index)?;

        require!(args.total_payout <= MAX_PAYOUT_LAMPORTS, CasinoErr::BadPayout);
        if args.total_payout > 0 {
            let bump_v = ctx.bumps.house_vault;
            let ix = system_instruction::transfer(&ctx.accounts.house_vault.key(), &ctx.accounts.user_vault.key(), args.total_payout);
            invoke_signed(&ix, &[
                ctx.accounts.house_vault.to_account_info(),
                ctx.accounts.user_vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ], &[&[b"vault", &[bump_v]]])?;
        }
        p.settled = true;
        emit!(PlinkoResolved { player: p.player, total_payout: args.total_payout, checksum: args.checksum, nonce: p.nonce });
        Ok(())
    }

    // ---- slots ----
    #[derive(Accounts)]
    #[instruction(args: SlotsLockArgs)]
    pub struct SlotsLock<'info> {
        pub player: SystemAccount<'info>,
        #[account(mut, signer)] pub fee_payer: SystemAccount<'info>,
        #[account(mut, seeds=[b"user_vault", player.key().as_ref()], bump=user_vault.bump)]
        pub user_vault: Account<'info, UserVault>,
        #[account(mut, seeds=[b"vault"], bump)]
        pub house_vault: SystemAccount<'info>,
        #[account(init, payer=fee_payer, space=PendingSlots::LEN, seeds=[b"spin", player.key().as_ref(), &args.nonce.to_le_bytes()], bump)]
        pub pending: Account<'info, PendingSlots>,
        pub system_program: Program<'info, System>,
        /// CHECK
        #[account(address = SYSVAR_INSTRUCTIONS_ID)]
        pub sysvar_instructions: UncheckedAccount<'info>,
    }
    #[derive(Accounts)]
    pub struct SlotsResolve<'info> {
        #[account(mut)] pub player: SystemAccount<'info>,
        #[account(mut, seeds=[b"vault"], bump)] pub house_vault: SystemAccount<'info>,
        #[account(seeds=[b"admin"], bump)] pub admin_config: Account<'info, AdminConfig>,
        #[account(mut, seeds=[b"user_vault", player.key().as_ref()], bump=user_vault.bump)]
        pub user_vault: Account<'info, UserVault>,
        #[account(mut, close=user_vault, seeds=[b"spin", player.key().as_ref(), &pending.nonce.to_le_bytes()], bump)]
        pub pending: Account<'info, PendingSlots>,
        pub system_program: Program<'info, System>,
        /// CHECK
        #[account(address = SYSVAR_INSTRUCTIONS_ID)]
        pub sysvar_instructions: UncheckedAccount<'info>,
    }

    pub fn slots_lock(ctx: Context<SlotsLock>, args: SlotsLockArgs) -> Result<()> {
        require!(args.bet_amount >= MIN_BET_LAMPORTS && args.bet_amount <= MAX_BET_LAMPORTS, CasinoErr::BadParams);
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
        p.player = ctx.accounts.player.key(); p.amount = args.bet_amount; p.nonce = args.nonce; p.expiry_unix = args.expiry_unix; p.settled = false;
        emit!(SlotsLocked { player: p.player, amount: p.amount, nonce: p.nonce });
        Ok(())
    }

    pub fn slots_resolve(ctx: Context<SlotsResolve>, args: SlotsResolveArgs) -> Result<()> {
        let p = &mut ctx.accounts.pending;
        require!(!p.settled, CasinoErr::BadPending);

        let clock = Clock::get()?;
        require!(clock.unix_timestamp <= p.expiry_unix, CasinoErr::Expired);
        require_ed25519_present(&ctx.accounts.sysvar_instructions.to_account_info(), args.ed25519_instr_index)?;

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
        emit!(SlotsResolved { player: p.player, payout: args.payout, checksum: args.checksum, nonce: p.nonce });
        Ok(())
    }
}
