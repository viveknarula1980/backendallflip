// use anchor_lang::system_program;
// use anchor_lang::prelude::*;
// use anchor_lang::solana_program::{
//     ed25519_program,
//     instruction::Instruction,
//     program::{invoke, invoke_signed},
//     system_instruction,
//     sysvar::instructions::{
//         load_current_index_checked,
//         load_instruction_at_checked,
//         ID as SYSVAR_INSTRUCTIONS_ID,
//     },
// };

// /// 👇 set this to your CURRENT deployed program id
// declare_id!("2m2qnCreEkuSf1CCZmWvjyBgAWkDYF13quCvsyEkDzGT");

// // App rails
// const DOMAIN_TAG: &[u8] = b"DICE_V1";
// const MAX_PAYOUT_LAMPORTS: u64 = 50_000_000_000; // 0.05 SOL
// const MIN_BET_LAMPORTS: u64 = 50_000;            // 0.00005 SOL
// const MAX_BET_LAMPORTS: u64 = 5_000_000_000;     // 5 SOL

// #[error_code]
// pub enum DiceError {
//     #[msg("Invalid instruction data")] InvalidIx,
//     #[msg("Invalid ed25519 pre-instruction")] InvalidEd25519,
//     #[msg("Expired signature")] Expired,
//     #[msg("Bet params invalid")] BadParams,
//     #[msg("Payout sanity check failed")] BadPayout,
//     #[msg("Vault mismatch")] VaultMismatch,
//     #[msg("Bet not found or already settled")] BadBet,
// }

// #[account]
// pub struct AdminConfig {
//     /// Trusted ed25519 public key (32 bytes) of your backend signer
//     pub admin_pubkey: [u8; 32],
// }

// #[account]
// pub struct PendingBet {
//     pub player: Pubkey,
//     pub amount: u64,
//     pub bet_type: u8, // 0 under, 1 over
//     pub target: u8,   // 2..98
//     pub nonce: u64,
//     pub expiry_unix: i64,
//     pub settled: bool,
// }
// impl PendingBet {
//     // 8 (disc) + 32 + 8 + 1 + 1 + 8 + 8 + 1 = 67
//     pub const LEN: usize = 8 + 32 + 8 + 1 + 1 + 8 + 8 + 1;
// }

// #[derive(Accounts)]
// pub struct InitAdmin<'info> {
//     #[account(mut, signer)]
//     pub authority: SystemAccount<'info>,

//     #[account(
//         init,
//         payer = authority,
//         space = 8 + 32,
//         seeds = [b"admin"],
//         bump
//     )]
//     pub admin_config: Account<'info, AdminConfig>,

//     pub system_program: Program<'info, System>,
// }

// #[derive(Accounts)]
// pub struct InitVault<'info> {
//     #[account(mut, signer)]
//     pub payer: SystemAccount<'info>,

//     // Will be created as a system account via CPI
//     #[account(mut, seeds = [b"vault"], bump)]
//     /// CHECK: created as system account; validated by seeds
//     pub vault: UncheckedAccount<'info>,

//     pub system_program: Program<'info, System>,
// }

// #[derive(Accounts)]
// #[instruction(args: PlaceBetLockArgs)]
// pub struct PlaceBetLock<'info> {
//     /// Player (signer) — will deposit bet_amount
//     #[account(mut, signer)]
//     pub player: SystemAccount<'info>,

//     /// Vault PDA (native SOL). Program holds funds here.
//     #[account(mut, seeds = [b"vault"], bump)]
//     pub vault: SystemAccount<'info>,

//     /// Pending bet PDA
//     #[account(
//         init,
//         payer = player,
//         space = PendingBet::LEN,
//         seeds = [b"bet", player.key().as_ref(), &args.nonce.to_le_bytes()],
//         bump
//     )]
//     pub pending_bet: Account<'info, PendingBet>,

//     pub system_program: Program<'info, System>,
// }

// #[derive(Accounts)]
// pub struct ResolveBet<'info> {
//     /// Player (not required to sign for resolution)
//     #[account(mut)]
//     pub player: SystemAccount<'info>,

//     /// Vault PDA
//     #[account(mut, seeds = [b"vault"], bump)]
//     pub vault: SystemAccount<'info>,

//     /// Admin config PDA containing backend ed25519 pubkey
//     #[account(seeds = [b"admin"], bump)]
//     pub admin_config: Account<'info, AdminConfig>,

//     /// Pending bet PDA to resolve (refund rent to player on success)
//     #[account(
//         mut,
//         seeds = [b"bet", player.key().as_ref(), &pending_bet.nonce.to_le_bytes()],
//         bump,
//         close = player
//     )]
//     pub pending_bet: Account<'info, PendingBet>,

//     pub system_program: Program<'info, System>,

//     /// CHECK: Sysvar Instructions account used to read the ed25519 verify pre-instruction
//     #[account(address = SYSVAR_INSTRUCTIONS_ID)]
//     pub sysvar_instructions: UncheckedAccount<'info>,
// }

// #[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
// pub struct PlaceBetLockArgs {
//     pub bet_amount: u64,
//     pub bet_type: u8,     // 0=under, 1=over
//     pub target: u8,       // 2..98
//     pub nonce: u64,
//     pub expiry_unix: i64, // unix seconds
// }

// #[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
// pub struct ResolveBetArgs {
//     pub roll: u8,                 // 1..100  (backend RNG)
//     pub payout: u64,              // 0 if loss; >0 if win (net)
//     pub ed25519_instr_index: u8,  // index hint of ed25519 verify ix
// }

// #[event]
// pub struct BetLocked {
//     pub player: Pubkey,
//     pub amount: u64,
//     pub bet_type: u8,
//     pub target: u8,
//     pub nonce: u64,
// }

// #[event]
// pub struct BetResolved {
//     pub player: Pubkey,
//     pub win: bool,
//     pub roll: u8,
//     pub payout: u64,
//     pub nonce: u64,
// }

// #[program]
// pub mod anchor_dice {
//     use super::*;

//     pub fn init_admin(ctx: Context<InitAdmin>, admin_pubkey: [u8; 32]) -> Result<()> {
//         ctx.accounts.admin_config.admin_pubkey = admin_pubkey;
//         Ok(())
//     }

//     pub fn init_vault(ctx: Context<InitVault>) -> Result<()> {
//         // Create the vault PDA as a system account with minimal lamports
//         let rent = Rent::get()?.minimum_balance(0);
//         let lamports = rent.max(1); // at least 1 lamport

//         let bump = ctx.bumps.vault;
//         let create_ix = system_instruction::create_account(
//             &ctx.accounts.payer.key(),
//             &ctx.accounts.vault.key(),
//             lamports,
//             0, // space
//             &system_program::ID, // owner
//         );

//         invoke_signed(
//             &create_ix,
//             &[
//                 ctx.accounts.payer.to_account_info(),
//                 ctx.accounts.vault.to_account_info(),
//                 ctx.accounts.system_program.to_account_info(),
//             ],
//             &[&[b"vault", &[bump]]],
//         )?;

//         Ok(())
//     }

//     /// Step 1: Player deposits bet into vault and opens a PendingBet
//     pub fn place_bet_lock(ctx: Context<PlaceBetLock>, args: PlaceBetLockArgs) -> Result<()> {
//         require!(
//             args.bet_amount >= MIN_BET_LAMPORTS && args.bet_amount <= MAX_BET_LAMPORTS,
//             DiceError::BadParams
//         );
//         require!(args.target >= 2 && args.target <= 98, DiceError::BadParams);
//         require!(args.bet_type <= 1, DiceError::BadParams);

//         // Transfer player → vault
//         let collect_ix = system_instruction::transfer(
//             &ctx.accounts.player.key(),
//             &ctx.accounts.vault.key(),
//             args.bet_amount,
//         );
//         invoke(
//             &collect_ix,
//             &[
//                 ctx.accounts.player.to_account_info(),
//                 ctx.accounts.vault.to_account_info(),
//                 ctx.accounts.system_program.to_account_info(),
//             ],
//         )?;

//         // Record pending bet
//         let pb = &mut ctx.accounts.pending_bet;
//         pb.player = ctx.accounts.player.key();
//         pb.amount = args.bet_amount;
//         pb.bet_type = args.bet_type;
//         pb.target = args.target;
//         pb.nonce = args.nonce;
//         pb.expiry_unix = args.expiry_unix;
//         pb.settled = false;

//         emit!(BetLocked {
//             player: pb.player,
//             amount: pb.amount,
//             bet_type: pb.bet_type,
//             target: pb.target,
//             nonce: pb.nonce,
//         });

//         Ok(())
//     }

//     /// Step 2: Backend signs result. Program verifies pre-instruction + rails, then pays if win
//     pub fn resolve_bet(ctx: Context<ResolveBet>, args: ResolveBetArgs) -> Result<()> {
//         let pb = &mut ctx.accounts.pending_bet;
//         require!(!pb.settled, DiceError::BadBet);

//         // Expiry
//         let clock = Clock::get()?;
//         require!(clock.unix_timestamp <= pb.expiry_unix, DiceError::Expired);

//         // --- Robust ed25519 presence check (index hint + fallback scan) ---
//         let sys_ix_ai = &ctx.accounts.sysvar_instructions.to_account_info();

//         let hinted_ok = load_instruction_at_checked(args.ed25519_instr_index as usize, sys_ix_ai)
//             .map(|ix| ix.program_id == ed25519_program::id())
//             .unwrap_or(false);

//         let mut found = hinted_ok;
//         if !found {
//             let cur_idx = load_current_index_checked(sys_ix_ai)?; // index of THIS instruction
//             for i in 0..cur_idx {
//                 if let Ok(ix) = load_instruction_at_checked(i as usize, sys_ix_ai) {
//                     if ix.program_id == ed25519_program::id() {
//                         found = true;
//                         break;
//                     }
//                 }
//             }
//         }
//         require!(found, DiceError::InvalidEd25519);

//         // Outcome + payout rails
//         require!(args.roll >= 1 && args.roll <= 100, DiceError::BadParams);
//         let win = match pb.bet_type {
//             0 => args.roll < pb.target, // under
//             _ => args.roll > pb.target, // over
//         };
//         if win {
//             require!(args.payout > 0 && args.payout <= MAX_PAYOUT_LAMPORTS, DiceError::BadPayout);
//         } else {
//             require!(args.payout == 0, DiceError::BadPayout);
//         }

//         // Pay winnings from vault → player
//         if win && args.payout > 0 {
//             let payout_ix = system_instruction::transfer(
//                 &ctx.accounts.vault.key(),
//                 &ctx.accounts.player.key(),
//                 args.payout,
//             );
//             let bump = ctx.bumps.vault;
//             let seeds: &[&[u8]] = &[b"vault", &[bump]];
//             invoke_signed(
//                 &payout_ix,
//                 &[
//                     ctx.accounts.vault.to_account_info(),
//                     ctx.accounts.player.to_account_info(),
//                     ctx.accounts.system_program.to_account_info(),
//                 ],
//                 &[seeds],
//             )?;
//         }

//         // Mark settled (account closes to player at end of ix due to `close = player`)
//         pb.settled = true;

//         emit!(BetResolved {
//             player: pb.player,
//             win,
//             roll: args.roll,
//             payout: args.payout,
//             nonce: pb.nonce,
//         });

//         Ok(())
//     }
// }

// user vault //
// 

use anchor_lang::system_program;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    program::invoke,
    system_instruction,
};

declare_id!("EwaLevizayb9FWC4sn7PSLBnubcvKd3s2rLY8XAzHwe");

// ---------- config ----------
const DOMAIN_TAG: &[u8] = b"DICE_V1";

// 0.05 SOL
const MAX_PAYOUT_LAMPORTS: u64 = 5_000_000_000;
// 0.00005 SOL
const MIN_BET_LAMPORTS: u64 = 50_000;
// 5 SOL
const MAX_BET_LAMPORTS: u64 = 5_000_000_000;
// flat fee reimbursement from user vault to fee-payer per bet lock (0.00001 SOL)
const FEE_REIMBURSE_LAMPORTS: u64 = 10_000;

// ---------- errors ----------
#[error_code]
pub enum DiceError {
    #[msg("Invalid instruction data")] InvalidIx,
    #[msg("Expired signature")] Expired,
    #[msg("Bet params invalid")] BadParams,
    #[msg("Payout sanity check failed")] BadPayout,
    #[msg("Vault mismatch")] VaultMismatch,
    #[msg("Bet not found or already settled")] BadBet,
    #[msg("Insufficient vault balance")] InsufficientVault,
    #[msg("Unauthorized")] Unauthorized,
}

// ---------- accounts ----------
#[account]
pub struct AdminConfig {
    pub admin_pubkey: [u8; 32],
}

#[account]
pub struct HouseVault {
    pub bump: u8,
}
impl HouseVault {
    pub const LEN: usize = 8 + 1;
}

#[account]
pub struct UserVault {
    pub owner: Pubkey,
    pub bump: u8,
    pub _rsv1: [u8; 7],
    pub _rsv2: [u8; 32],
    pub _rsv3: i64,
    pub _rsv4: u64,
    pub _rsv5: u64,
}
impl UserVault {
    pub const LEN: usize = 8 + 32 + 1 + 7 + 32 + 8 + 8 + 8;
}

#[account]
pub struct PendingBet {
    pub player: Pubkey,
    pub amount: u64,
    pub bet_type: u8,
    pub target: u8,
    pub nonce: u64,
    pub expiry_unix: i64,
    pub settled: bool,
}
impl PendingBet {
    pub const LEN: usize = 8 + 32 + 8 + 1 + 1 + 8 + 8 + 1;
}

// ---------- contexts ----------
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
pub struct InitHouseVault<'info> {
    #[account(mut, signer)]
    pub payer: SystemAccount<'info>,
    #[account(
        init,
        payer = payer,
        space = HouseVault::LEN,
        seeds = [b"vault"],
        bump
    )]
    pub house_vault: Account<'info, HouseVault>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ActivateUserVault<'info> {
    #[account(mut, signer)]
    pub player: SystemAccount<'info>,
    #[account(
        init,
        payer = player,
        space = UserVault::LEN,
        seeds = [b"user_vault", player.key().as_ref()],
        bump
    )]
    pub user_vault: Account<'info, UserVault>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct DepositToVault<'info> {
    #[account(mut, signer)]
    pub player: SystemAccount<'info>,
    #[account(
        mut,
        seeds = [b"user_vault", player.key().as_ref()],
        bump = user_vault.bump
    )]
    pub user_vault: Account<'info, UserVault>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawFromVault<'info> {
    #[account(mut, signer)]
    pub player: SystemAccount<'info>,
    #[account(
        mut,
        seeds = [b"user_vault", player.key().as_ref()],
        bump = user_vault.bump
    )]
    pub user_vault: Account<'info, UserVault>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(args: PlaceBetFromVaultArgs)]
pub struct PlaceBetFromVault<'info> {
    /// Any system account representing the player (not required to sign here)
    pub player: SystemAccount<'info>,

    /// Server fee-payer (must be the configured admin)
    #[account(mut, signer)]
    pub fee_payer: SystemAccount<'info>,

    #[account(
        seeds = [b"admin"],
        bump
    )]
    pub admin_config: Account<'info, AdminConfig>,

    #[account(
        mut,
        seeds = [b"user_vault", player.key().as_ref()],
        bump = user_vault.bump
    )]
    pub user_vault: Account<'info, UserVault>,

    #[account(
        mut,
        seeds = [b"vault"],
        bump = house_vault.bump
    )]
    pub house_vault: Account<'info, HouseVault>,

    #[account(
        init,
        payer = fee_payer,
        space = PendingBet::LEN,
        seeds = [b"bet", player.key().as_ref(), &args.nonce.to_le_bytes()],
        bump
    )]
    pub pending_bet: Account<'info, PendingBet>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ResolveBet<'info> {
    /// Player (not required to sign)
    pub player: SystemAccount<'info>,

    #[account(
        seeds = [b"admin"],
        bump
    )]
    pub admin_config: Account<'info, AdminConfig>,

    /// Admin authority must sign resolutions
    #[account(mut, signer)]
    pub authority: SystemAccount<'info>,

    #[account(
        mut,
        seeds = [b"vault"],
        bump = house_vault.bump
    )]
    pub house_vault: Account<'info, HouseVault>,

    #[account(
        mut,
        seeds = [b"user_vault", player.key().as_ref()],
        bump = user_vault.bump
    )]
    pub user_vault: Account<'info, UserVault>,

    #[account(
        mut,
        seeds = [b"bet", player.key().as_ref(), &pending_bet.nonce.to_le_bytes()],
        bump,
        close = user_vault
    )]
    pub pending_bet: Account<'info, PendingBet>,

    pub system_program: Program<'info, System>,
}

// ---------- args ----------
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct ActivateArgs {
    pub initial_deposit: u64,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct DepositArgs {
    pub amount: u64,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct WithdrawArgs {
    pub amount: u64,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct PlaceBetFromVaultArgs {
    pub bet_amount: u64,
    pub bet_type: u8,
    pub target: u8,
    pub nonce: u64,
    pub expiry_unix: i64,
    // kept for compatibility; unused after moving to signer gating
    pub ed25519_instr_index: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct ResolveBetArgs {
    pub roll: u8,
    pub payout: u64,
    // kept for compatibility; unused after moving to signer gating
    pub ed25519_instr_index: u8,
}

// ---------- events ----------
#[event]
pub struct BetLocked {
    pub player: Pubkey,
    pub amount: u64,
    pub bet_type: u8,
    pub target: u8,
    pub nonce: u64,
}

#[event]
pub struct BetResolved {
    pub player: Pubkey,
    pub win: bool,
    pub roll: u8,
    pub payout: u64,
    pub nonce: u64,
}

// ---------- helpers ----------
fn safe_move_lamports(from: &AccountInfo<'_>, to: &AccountInfo<'_>, amount: u64) -> Result<()> {
    require!(amount > 0, DiceError::BadParams);
    let mut from_lamports = from.try_borrow_mut_lamports()?;
    let mut to_lamports = to.try_borrow_mut_lamports()?;
    require!(**from_lamports >= amount, DiceError::InsufficientVault);
    **from_lamports -= amount;
    **to_lamports += amount;
    Ok(())
}

// ---------- program ----------
#[program]
pub mod anchor_dice {
    use super::*;

    pub fn init_admin(ctx: Context<InitAdmin>, admin_pubkey: [u8; 32]) -> Result<()> {
        // Optional: require the initializer to be the admin they're setting
        let admin_pk = Pubkey::new_from_array(admin_pubkey);
        require_keys_eq!(ctx.accounts.authority.key(), admin_pk, DiceError::Unauthorized);

        ctx.accounts.admin_config.admin_pubkey = admin_pubkey;
        Ok(())
    }

    pub fn init_house_vault(ctx: Context<InitHouseVault>) -> Result<()> {
        ctx.accounts.house_vault.bump = ctx.bumps.house_vault;
        Ok(())
    }

    pub fn activate_user_vault(ctx: Context<ActivateUserVault>, args: ActivateArgs) -> Result<()> {
        let uv = &mut ctx.accounts.user_vault;
        uv.owner = ctx.accounts.player.key();
        uv.bump = ctx.bumps.user_vault;

        if args.initial_deposit > 0 {
            let collect_ix = system_instruction::transfer(
                &ctx.accounts.player.key(),
                &ctx.accounts.user_vault.key(),
                args.initial_deposit,
            );
            invoke(
                &collect_ix,
                &[
                    ctx.accounts.player.to_account_info(),
                    ctx.accounts.user_vault.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }
        Ok(())
    }

    pub fn deposit_to_vault(ctx: Context<DepositToVault>, args: DepositArgs) -> Result<()> {
        require!(args.amount > 0, DiceError::BadParams);
        let collect_ix = system_instruction::transfer(
            &ctx.accounts.player.key(),
            &ctx.accounts.user_vault.key(),
            args.amount,
        );
        invoke(
            &collect_ix,
            &[
                ctx.accounts.player.to_account_info(),
                ctx.accounts.user_vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;
        Ok(())
    }

    pub fn withdraw_from_vault(ctx: Context<WithdrawFromVault>, args: WithdrawArgs) -> Result<()> {
        require!(args.amount > 0, DiceError::BadParams);
        // Optional: protect rent; comment out if you don't want it
        // let rent_min = Rent::get()?.minimum_balance(UserVault::LEN);
        // let uv_lamports = **ctx.accounts.user_vault.to_account_info().lamports.borrow();
        // require!(uv_lamports.saturating_sub(args.amount) >= rent_min, DiceError::InsufficientVault);

        let from_ai = ctx.accounts.user_vault.to_account_info();
        let to_ai = ctx.accounts.player.to_account_info();
        safe_move_lamports(&from_ai, &to_ai, args.amount)?;
        Ok(())
    }

    pub fn place_bet_from_vault(ctx: Context<PlaceBetFromVault>, args: PlaceBetFromVaultArgs) -> Result<()> {
        // --- admin signer gate ---
        let admin = Pubkey::new_from_array(ctx.accounts.admin_config.admin_pubkey);
        require_keys_eq!(ctx.accounts.fee_payer.key(), admin, DiceError::Unauthorized);

        // --- basic param checks ---
        require!(args.bet_amount >= MIN_BET_LAMPORTS && args.bet_amount <= MAX_BET_LAMPORTS, DiceError::BadParams);
        require!(args.target >= 2 && args.target <= 98, DiceError::BadParams);
        require!(args.bet_type <= 1, DiceError::BadParams);
        require!(ctx.accounts.user_vault.owner == ctx.accounts.player.key(), DiceError::VaultMismatch);

        // ensure user vault covers bet + reimbursement
        let uv_lamports = **ctx.accounts.user_vault.to_account_info().lamports.borrow();
        let total_need = args.bet_amount.saturating_add(FEE_REIMBURSE_LAMPORTS);
        require!(uv_lamports >= total_need, DiceError::InsufficientVault);

        // move bet -> house vault
        let uv_ai = ctx.accounts.user_vault.to_account_info();
        let hv_ai = ctx.accounts.house_vault.to_account_info();
        safe_move_lamports(&uv_ai, &hv_ai, args.bet_amount)?;

        // reimburse fee-payer (optional)
        if FEE_REIMBURSE_LAMPORTS > 0 {
            let fp_ai = ctx.accounts.fee_payer.to_account_info();
            safe_move_lamports(&uv_ai, &fp_ai, FEE_REIMBURSE_LAMPORTS)?;
        }

        // record pending bet
        let pb = &mut ctx.accounts.pending_bet;
        pb.player = ctx.accounts.player.key();
        pb.amount = args.bet_amount;
        pb.bet_type = args.bet_type;
        pb.target = args.target;
        pb.nonce = args.nonce;
        pb.expiry_unix = args.expiry_unix;
        pb.settled = false;

        emit!(BetLocked {
            player: pb.player,
            amount: pb.amount,
            bet_type: pb.bet_type,
            target: pb.target,
            nonce: pb.nonce,
        });

        Ok(())
    }

    pub fn resolve_bet(ctx: Context<ResolveBet>, args: ResolveBetArgs) -> Result<()> {
        // --- admin signer gate ---
        let admin = Pubkey::new_from_array(ctx.accounts.admin_config.admin_pubkey);
        require_keys_eq!(ctx.accounts.authority.key(), admin, DiceError::Unauthorized);

        // --- fetch and validate bet ---
        let pb = &mut ctx.accounts.pending_bet;
        require!(!pb.settled, DiceError::BadBet);

        let clock = Clock::get()?;
        require!(clock.unix_timestamp <= pb.expiry_unix, DiceError::Expired);

        // --- game outcome checks ---
        require!(args.roll >= 1 && args.roll <= 100, DiceError::BadParams);
        let win = match pb.bet_type {
            0 => args.roll < pb.target,
            _ => args.roll > pb.target,
        };
        if win {
            require!(args.payout > 0 && args.payout <= MAX_PAYOUT_LAMPORTS, DiceError::BadPayout);
            // Optional: on-chain RTP sanity (example cap ~99% RTP)
            // require!(args.payout <= pb.amount.saturating_mul(9900) / 10000, DiceError::BadPayout);
        } else {
            require!(args.payout == 0, DiceError::BadPayout);
        }

        // --- settle payout (house_vault -> user_vault) ---
        if win && args.payout > 0 {
            let hv_ai = ctx.accounts.house_vault.to_account_info();
            let uv_ai = ctx.accounts.user_vault.to_account_info();
            safe_move_lamports(&hv_ai, &uv_ai, args.payout)?;
        }

        pb.settled = true;

        emit!(BetResolved {
            player: pb.player,
            win,
            roll: args.roll,
            payout: args.payout,
            nonce: pb.nonce,
        });

        Ok(())
    }
}
