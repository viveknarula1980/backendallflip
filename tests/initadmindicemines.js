// scripts/init_admin_vault.js
// Initializes AdminConfig (seeds ["admin"]) and creates the System-owned House Vault PDA (seeds ["vault"])
// Compatible with your new on-chain program (init_admin, init_house_vault)

const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const anchor = require("@coral-xyz/anchor");
const {
  PublicKey,
  Connection,
  Keypair,
  SystemProgram,
  Transaction,
  TransactionInstruction,
  sendAndConfirmTransaction,
} = require("@solana/web3.js");

// --- CONFIG ---
const RPC_URL =
  process.env.RPC_URL || "https://api.devnet.solana.com";
const WALLET_PATH =
  process.env.WALLET_PATH || path.join(process.env.HOME, ".config/solana/id.json");

// Default to your new program id (can be overridden via env)
const PROGRAM_ID = new PublicKey(
  process.env.PROGRAM_ID || "5vgLU8GyehUkziMaKHCtyPu6YZgo11wct8rTHLdz4z1"
);

// Backend admin **PUBLIC** key (base58, 32 bytes)
const ADMIN_PUBKEY_BASE58 =
  process.env.ADMIN_PUBKEY_BASE58 ||
  "EWaMqbKeyv2V2WheLUDuuFs7DqTqhARVd34JvkZPRu7z";

// Common IDL locations to try
const IDL_CANDIDATES = [
  path.join(__dirname, "../target/idl/casino.json"),
  path.join(process.cwd(), "target/idl/casino.json"),
  path.join(__dirname, "../idl/casino.json"),
];

// --- UTILS ---
function anchorDiscriminator(ixName) {
  const preimage = Buffer.from(`global:${ixName}`);
  return crypto.createHash("sha256").update(preimage).digest().subarray(0, 8);
}

function loadIdlOrNull() {
  for (const p of IDL_CANDIDATES) {
    if (fs.existsSync(p)) {
      try {
        const idl = JSON.parse(fs.readFileSync(p, "utf8"));
        return { idl, path: p };
      } catch (_) {}
    }
  }
  return { idl: null, path: null };
}

(async () => {
  // --- Load payer ---
  const payer = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(fs.readFileSync(WALLET_PATH, "utf8")))
  );

  const connection = new Connection(RPC_URL, "confirmed");

  // --- Load IDL (optional but preferred) ---
  const { idl, path: idlPath } = loadIdlOrNull();
  if (idl) {
    console.log("IDL loaded from:", idlPath);
    console.log("IDL name:", idl.name);
    console.log("IDL instructions:", (idl.instructions || []).map((i) => i.name));
  } else {
    console.warn(
      "IDL not found (will use manual encoding fallback). Searched:",
      IDL_CANDIDATES
    );
  }

  // --- Derive PDAs (must match on-chain seeds) ---
  const [adminPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("admin")],
    PROGRAM_ID
  );
  const [houseVault] = PublicKey.findProgramAddressSync(
    [Buffer.from("vault")],
    PROGRAM_ID
  );

  // --- Validate ADMIN pubkey (must be 32 bytes) ---
  const adminU8 = anchor.utils.bytes.bs58.decode(ADMIN_PUBKEY_BASE58);
  if (adminU8.length !== 32) {
    throw new Error(
      `ADMIN_PUBKEY_BASE58 must decode to 32 bytes (PUBLIC key). Got ${adminU8.length}.`
    );
  }

  // ======================
  // 1) init_admin
  // ======================
  let dataInitAdmin;
  if (idl) {
    try {
      const coder = new anchor.BorshCoder(idl);
      const names = (idl.instructions || []).map((i) => i.name);
      // Your program uses `init_admin` in Rust; Anchor IDL typically camelCases arg names
      // Try both `init_admin` and `initAdmin` for safety
      for (const nm of ["init_admin", "initAdmin"]) {
        if (names.includes(nm)) {
          dataInitAdmin = coder.instruction.encode(nm, {
            adminPubkey: Array.from(adminU8),
          });
          console.log(`Using IDL coder for instruction: ${nm}`);
          break;
        }
      }
    } catch (e) {
      console.warn("IDL coder path failed for init_admin, fallback to manual:", e.message);
    }
  }
  if (!dataInitAdmin) {
    // manual: 8-byte discriminator + [u8;32] admin_pubkey
    const disc = anchorDiscriminator("init_admin");
    dataInitAdmin = Buffer.concat([disc, Buffer.from(adminU8)]);
  }

  const ixInitAdmin = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      // #[account(mut, signer)] pub authority: SystemAccount<'info>,
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      // #[account(init, payer=authority, space=8+32, seeds=[b"admin"], bump)]
      { pubkey: adminPda, isSigner: false, isWritable: true },
      // pub system_program: Program<'info, System>,
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: dataInitAdmin,
  });

  // ======================
  // 2) init_house_vault
  // ======================
  let dataInitVault;
  if (idl) {
    try {
      const coder = new anchor.BorshCoder(idl);
      const names = (idl.instructions || []).map((i) => i.name);
      for (const nm of ["init_house_vault", "initHouseVault"]) {
        if (names.includes(nm)) {
          dataInitVault = coder.instruction.encode(nm, {}); // no args
          console.log(`Using IDL coder for instruction: ${nm}`);
          break;
        }
      }
    } catch (e) {
      console.warn("IDL coder path failed for init_house_vault, fallback to manual:", e.message);
    }
  }
  if (!dataInitVault) {
    // manual: 8-byte discriminator only
    const disc = anchorDiscriminator("init_house_vault");
    dataInitVault = Buffer.from(disc);
  }

  const ixInitVault = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      // #[account(mut, signer)] pub payer: SystemAccount<'info>,
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      // #[account(mut, seeds=[b"vault"], bump)] pub house_vault: UncheckedAccount<'info>,
      { pubkey: houseVault, isSigner: false, isWritable: true },
      // pub system_program: Program<'info, System>,
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: dataInitVault,
  });

  // --- Send both in a single tx ---
  const tx = new Transaction().add(ixInitAdmin).add(ixInitVault);
  const sig = await sendAndConfirmTransaction(connection, tx, [payer], {
    commitment: "confirmed",
  });

  console.log("✅ init_admin + init_house_vault signature:", sig);
  console.log("Admin PDA:", adminPda.toBase58());
  console.log("House Vault PDA:", houseVault.toBase58());

  // --- Verify the vault system account now exists ---
  const vaultAcc = await connection.getAccountInfo(houseVault, "confirmed");
  if (vaultAcc) {
    console.log("House Vault lamports:", vaultAcc.lamports);
    console.log("House Vault owner:", vaultAcc.owner.toBase58());
    if (!vaultAcc.owner.equals(SystemProgram.programId)) {
      console.warn("⚠️ House Vault owner is not System Program (unexpected).");
    }
  } else {
    console.warn("⚠️ House Vault account not found (creation may have failed).");
  }
})().catch((e) => {
  console.error("FAILED:", e);
  console.error("Hints:");
  console.error(" • Anchor build to generate IDL (or rely on manual fallback)");
  console.error(" • ADMIN_PUBKEY_BASE58 must be a base58 PUBLIC key (32 bytes)");
  console.error(" • Wallet must have SOL for rent + fees");
  console.error(" • PROGRAM_ID must match the deployed program (declare_id)");
  process.exit(1);
});
