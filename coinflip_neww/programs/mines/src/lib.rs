use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};
use anchor_lang::solana_program::sysvar;

declare_id!("GMngB79rVBvjrQFEF7b8dnkk7xr3xZQc3kZnQSXNsao2"); // ⬅️ REPLACE with your REAL deployed program id

#[program]
pub mod mines {
    use super::*;

    /// One-time initializer: creates the Vault PDA (lamports-only account).
    /// Accounts:
    /// - authority: signer (pays rent)
    /// - vault: PDA ["vault"]
    /// - system_program
    pub fn initialize(_ctx: Context<Initialize>) -> Result<()> {
        Ok(())
    }

    /// User-paid lock: moves bet lamports into the vault and records the pending round.
    /// Accounts:
    /// - player: signer
    /// - vault: PDA ["vault"] (MUST exist; created via `initialize`)
    /// - pending: PDA ["round", player, nonce_le_u64]
    /// - system_program
    pub fn lock(
        ctx: Context<Lock>,
        bet_lamports: u64,
        rows: u8,
        cols: u8,
        mines: u8,
        nonce: u64,
        expiry_unix: i64,
    ) -> Result<()> {
        require!(bet_lamports > 0, MinesError::BadBet);
        require!(rows >= 2 && rows <= 8, MinesError::BadBoard);
        require!(cols >= 2 && cols <= 8, MinesError::BadBoard);

        let total = (rows as u16) * (cols as u16);
        require!(mines >= 1 && (mines as u16) < total, MinesError::BadMines);

        // move stake into vault PDA
        let from = ctx.accounts.player.to_account_info();
        let to = ctx.accounts.vault.to_account_info();
        let sys = ctx.accounts.system_program.to_account_info();
        transfer(CpiContext::new(sys, Transfer { from, to }), bet_lamports)?;

        // write round
        let pending = &mut ctx.accounts.pending;
        pending.player = ctx.accounts.player.key();
        pending.bet_lamports = bet_lamports;
        pending.rows = rows;
        pending.cols = cols;
        pending.mines = mines;
        pending.nonce = nonce;
        pending.expired_at = expiry_unix;
        pending.settled = false;

        Ok(())
    }

    /// Server-paid resolve: pays from vault PDA to player (backend decides payout).
    /// Accounts:
    /// - player: writable (receiver)
    /// - vault: PDA ["vault"] (signs with seeds to pay)
    /// - admin: unchecked (kept to match backend account array)
    /// - pending: round pda (closed to player)
    /// - system_program
    /// - instructions sysvar (kept for compatibility with your WS/ed25519 flow; not enforced here)
    pub fn resolve(
        ctx: Context<Resolve>,
        checksum: u8,
        payout: u64,
        _ed25519_instr_index: u8,
    ) -> Result<()> {
        let pending = &mut ctx.accounts.pending;

        // light sanity checks aligned with backend
        let expected = ((pending.nonce % 251) + 1) as u8;
        require!(checksum == expected, MinesError::BadChecksum);
        require!(!pending.settled, MinesError::AlreadySettled);
        require!(pending.player == ctx.accounts.player.key(), MinesError::PlayerMismatch);

        // enforce correct vault PDA so signer seeds match
        let (vault_pda, vault_bump) = Pubkey::find_program_address(&[b"vault"], ctx.program_id);
        require!(vault_pda == ctx.accounts.vault.key(), MinesError::VaultMismatch);

        if payout > 0 {
            let signer_seeds: &[&[u8]] = &[b"vault", &[vault_bump]];
            let to = ctx.accounts.player.to_account_info();
            let from = ctx.accounts.vault.to_account_info();
            let sys = ctx.accounts.system_program.to_account_info();

            transfer(
                CpiContext::new_with_signer(sys, Transfer { from, to }, &[signer_seeds]),
                payout,
            )?;
        }

        // mark and close (rent refunded to player by `close = player`)
        pending.settled = true;
        Ok(())
    }
}

#[account]
pub struct Vault {} // Discriminator-only; holds lamports

#[account]
pub struct Pending {
    pub player: Pubkey,     // 32
    pub bet_lamports: u64,  // 8
    pub rows: u8,           // 1
    pub cols: u8,           // 1
    pub mines: u8,          // 1
    pub nonce: u64,         // 8
    pub expired_at: i64,    // 8
    pub settled: bool,      // 1
}
impl Pending {
    pub const SIZE: usize = 32 + 8 + 1 + 1 + 1 + 8 + 8 + 1; // 60
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8,                 // discriminator only
        seeds = [b"vault"],
        bump
    )]
    pub vault: Account<'info, Vault>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(bet_lamports: u64, rows: u8, cols: u8, mines: u8, nonce: u64, expiry_unix: i64)]
pub struct Lock<'info> {
    #[account(mut)]
    pub player: Signer<'info>,

    // MUST already exist (created by `initialize`)
    #[account(
        mut,
        seeds = [b"vault"],
        bump
    )]
    pub vault: Account<'info, Vault>,

    #[account(
        init,
        payer = player,
        space = 8 + Pending::SIZE,
        seeds = [b"round", player.key().as_ref(), &nonce.to_le_bytes()],
        bump
    )]
    pub pending: Account<'info, Pending>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Resolve<'info> {
    #[account(mut)]
    pub player: SystemAccount<'info>,

    #[account(mut, seeds = [b"vault"], bump)]
    pub vault: Account<'info, Vault>,

    /// CHECK: kept to match backend account array (unused in this instruction)
    pub admin: UncheckedAccount<'info>,

    #[account(
        mut,
        has_one = player,
        close = player,
        seeds = [b"round", player.key().as_ref(), &pending.nonce.to_le_bytes()],
        bump
    )]
    pub pending: Account<'info, Pending>,

    pub system_program: Program<'info, System>,

    /// CHECK: instructions sysvar (for optional ed25519 enforcement)
    #[account(address = sysvar::instructions::ID)]
    pub instructions: UncheckedAccount<'info>,
}

#[error_code]
pub enum MinesError {
    #[msg("Invalid bet amount")] BadBet,
    #[msg("Invalid board size")] BadBoard,
    #[msg("Invalid number of mines")] BadMines,
    #[msg("Checksum mismatch")] BadChecksum,
    #[msg("Round already settled")] AlreadySettled,
    #[msg("Pending player mismatch")] PlayerMismatch,
    #[msg("Vault PDA mismatch")] VaultMismatch,
}
