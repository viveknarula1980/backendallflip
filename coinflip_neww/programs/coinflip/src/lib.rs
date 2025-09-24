// programs/coinflip/src/lib.rs
use anchor_lang::system_program;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    program::{invoke, invoke_signed},
    system_instruction,
    sysvar,
};

declare_id!("2KR5B1jyJ3XwcWSDjRDZsbgMTNsh2PxyN1yULDS2oCQk");

#[program]
pub mod coinflip {
    use super::*;

    /// Create the vault PDA as a plain System account (space = 0).
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        // If it's already funded/created, do nothing
        if ctx.accounts.vault.lamports() > 0 {
            return Ok(());
        }

        // minimal lamports for a zero-space system account (at least 1 lamport)
        let rent_min = Rent::get()?.minimum_balance(0).max(1);

        let bump = ctx.bumps.vault;
        let ix = system_instruction::create_account(
            &ctx.accounts.authority.key(),
            &ctx.accounts.vault.key(),
            rent_min,
            0,                       // space
            &system_program::ID,     // owner = System Program
        );

        invoke_signed(
            &ix,
            &[
                ctx.accounts.authority.to_account_info(),
                ctx.accounts.vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[&[b"vault", &[bump]]],
        )?;

        Ok(())
    }

    /// Player deposits entry into vault and opens their pending round.
    pub fn lock(
        ctx: Context<Lock>,
        entry_lamports: u64,
        side: u8,         // 0=heads, 1=tails
        nonce: u64,       // shared match nonce
        expiry_unix: i64, // not enforced here
    ) -> Result<()> {
        require!(entry_lamports > 0, CfError::BadBet);
        require!(side <= 1, CfError::BadSide);

        // Transfer player → vault
        let ix = system_instruction::transfer(
            &ctx.accounts.player.key(),
            &ctx.accounts.vault.key(),
            entry_lamports,
        );
        invoke(
            &ix,
            &[
                ctx.accounts.player.to_account_info(),
                ctx.accounts.vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // Initialize pending
        let p = &mut ctx.accounts.pending;
        p.player = ctx.accounts.player.key();
        p.entry_lamports = entry_lamports;
        p.side = side;
        p.nonce = nonce;
        p.expired_at = expiry_unix;
        p.settled = false;

        Ok(())
    }

    /// Backend resolves: pays winner from vault; loser payout=0. Closes pending to player.
    pub fn resolve(
        ctx: Context<Resolve>,
        checksum: u8,         // must equal (nonce % 251) + 1
        payout: u64,          // lamports paid to this player
        _ed25519_ix_index: u8,
        winner_side: u8,      // 0=heads,1=tails (from backend RNG)
    ) -> Result<()> {
        let pending = &mut ctx.accounts.pending;

        // Rails
        let expected = ((pending.nonce % 251) + 1) as u8;
        require!(checksum == expected, CfError::BadChecksum);
        require!(!pending.settled, CfError::AlreadySettled);
        require!(pending.player == ctx.accounts.player.key(), CfError::PlayerMismatch);
        require!(winner_side <= 1, CfError::BadSide);

        if payout > 0 {
            require!(winner_side == pending.side, CfError::WrongWinnerSide);
        }

        // Confirm vault PDA
        let (vault_pda, vault_bump) = Pubkey::find_program_address(&[b"vault"], ctx.program_id);
        require!(vault_pda == ctx.accounts.vault.key(), CfError::VaultMismatch);

        // Pay winner from vault
        if payout > 0 {
            let ix = system_instruction::transfer(
                &ctx.accounts.vault.key(),
                &ctx.accounts.player.key(),
                payout,
            );
            invoke_signed(
                &ix,
                &[
                    ctx.accounts.vault.to_account_info(),
                    ctx.accounts.player.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
                &[&[b"vault", &[vault_bump]]],
            )?;
        }

        // mark settled; account closes to player via `close = player`
        pending.settled = true;
        Ok(())
    }
}

/* ---------------- Accounts ---------------- */

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault"],
        bump
    )]
    /// CHECK: Vault is created here as a zero-space System account via CPI; seeds validate the PDA.
    pub vault: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(entry_lamports: u64, side: u8, nonce: u64, expiry_unix: i64)]
pub struct Lock<'info> {
    #[account(mut)]
    pub player: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault"],
        bump
    )]
    /// CHECK: PDA address validated by seeds; funds are native SOL only.
    pub vault: UncheckedAccount<'info>,

    #[account(
        init,
        payer = player,
        space = 8 + Pending::SIZE,
        seeds = [b"match", player.key().as_ref(), &nonce.to_le_bytes()],
        bump
    )]
    pub pending: Account<'info, Pending>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Resolve<'info> {
    #[account(mut)]
    /// CHECK: Any wallet can be the receiver; key is compared with `pending.player`.
    pub player: UncheckedAccount<'info>,

    #[account(mut, seeds = [b"vault"], bump)]
    /// CHECK: PDA validated by seeds; used only for native SOL transfers.
    pub vault: UncheckedAccount<'info>,

    /// CHECK: Kept for backend layout compatibility; not used by the program.
    pub admin: UncheckedAccount<'info>,

    #[account(
        mut,
        has_one = player,
        close = player,
        seeds = [b"match", player.key().as_ref(), &pending.nonce.to_le_bytes()],
        bump
    )]
    pub pending: Account<'info, Pending>,

    pub system_program: Program<'info, System>,

    #[account(address = sysvar::instructions::ID)]
    /// CHECK: Sysvar Instructions account (optional ed25519 preverify by backend).
    pub instructions: UncheckedAccount<'info>,
}

/* ---------------- Data ---------------- */

#[account]
pub struct Pending {
    pub player: Pubkey,      // 32
    pub entry_lamports: u64, // 8
    pub side: u8,            // 1  (0=heads,1=tails)
    pub nonce: u64,          // 8
    pub expired_at: i64,     // 8
    pub settled: bool,       // 1
}
impl Pending {
    pub const SIZE: usize = 32 + 8 + 1 + 8 + 8 + 1; // 58
}

#[error_code]
pub enum CfError {
    #[msg("Invalid bet amount")] BadBet,
    #[msg("Invalid side (must be 0 or 1)")] BadSide,
    #[msg("Checksum mismatch")] BadChecksum,
    #[msg("Round already settled")] AlreadySettled,
    #[msg("Pending player mismatch")] PlayerMismatch,
    #[msg("Vault PDA mismatch")] VaultMismatch,
    #[msg("Winner side does not match player's chosen side")] WrongWinnerSide,
}
