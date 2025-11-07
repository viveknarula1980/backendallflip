// tests/coinflip_full_manual.js
require("dotenv").config();

const fs = require("fs");
const os = require("os");
const path = require("path");
const crypto = require("crypto");
const BN = require("bn.js");
const {
  Connection,
  PublicKey,
  Keypair,
  SystemProgram,
  TransactionMessage,
  VersionedTransaction,
  ComputeBudgetProgram,
} = require("@solana/web3.js");

/** ---- ENV ---- */
const RPC_URL =
  process.env.ANCHOR_PROVIDER_URL ||
  process.env.CLUSTER ||
  "https://api.devnet.solana.com";

if (!process.env.COINFLIP_PROGRAM_ID)
  throw new Error("Set COINFLIP_PROGRAM_ID=<Coinflip program id>");
if (!process.env.ANCHOR_WALLET)
  throw new Error("Set ANCHOR_WALLET=~/.config/solana/id.json");

const PROGRAM_ID = new PublicKey(process.env.COINFLIP_PROGRAM_ID);
const SYSVAR_INSTRUCTIONS = new PublicKey(
  "Sysvar1nstructions1111111111111111111111111"
);

/** ---- Load keypairs ---- */
function loadKeypair(filePath) {
  const abs = filePath.startsWith("~")
    ? path.join(os.homedir(), filePath.slice(1))
    : filePath;
  const secret = JSON.parse(fs.readFileSync(abs, "utf8"));
  return Keypair.fromSecretKey(Uint8Array.from(secret));
}
const admin = loadKeypair(process.env.ANCHOR_WALLET);

function loadUserKeypair() {
  const abs = path.join(os.homedir(), ".config", "solana", "user.json");
  if (!fs.existsSync(abs)) {
    throw new Error(
      `User keypair not found at ${abs}. Create with:\n` +
        `solana-keygen new --outfile ~/.config/solana/user.json`
    );
  }
  const secret = JSON.parse(fs.readFileSync(abs, "utf8"));
  return Keypair.fromSecretKey(Uint8Array.from(secret));
}
const user = loadUserKeypair();

/** ---- PDAs ---- */
const pdaVault = () =>
  PublicKey.findProgramAddressSync([Buffer.from("vault")], PROGRAM_ID)[0];
const pdaUserVault = (pk) =>
  PublicKey.findProgramAddressSync(
    [Buffer.from("user_vault"), pk.toBuffer()],
    PROGRAM_ID
  )[0];
const pdaAdminConfig = () =>
  PublicKey.findProgramAddressSync([Buffer.from("admin")], PROGRAM_ID)[0];
const pdaPending = (pk, nonce) =>
  PublicKey.findProgramAddressSync(
    [Buffer.from("match"), pk.toBuffer(), Buffer.from(new BN(nonce).toArray("le", 8))],
    PROGRAM_ID
  )[0];

/** ---- Anchor discriminator ---- */
const disc = (name) =>
  crypto.createHash("sha256").update(`global:${name}`).digest().slice(0, 8);

/** ---- Ixs ---- */
const ixInitAdmin = ({ authorityPk, adminConfigPk, adminBytes }) => ({
  programId: PROGRAM_ID,
  keys: [
    { pubkey: authorityPk, isSigner: true, isWritable: true },
    { pubkey: adminConfigPk, isSigner: false, isWritable: true },
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ],
  data: Buffer.concat([disc("init_admin"), Buffer.from(adminBytes)]),
});

const ixInitialize = ({ authorityPk, vaultPk }) => ({
  programId: PROGRAM_ID,
  keys: [
    { pubkey: authorityPk, isSigner: true, isWritable: true },
    { pubkey: vaultPk, isSigner: false, isWritable: true },
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ],
  data: disc("initialize"),
});

const ixInitUserVault = ({ playerPk, userVaultPk }) => ({
  programId: PROGRAM_ID,
  keys: [
    { pubkey: playerPk, isSigner: true, isWritable: true },
    { pubkey: userVaultPk, isSigner: false, isWritable: true },
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ],
  data: disc("init_user_vault"),
});

// NEW: deposit_user(amount)
const ixDepositUser = ({ playerPk, userVaultPk, amount }) => ({
  programId: PROGRAM_ID,
  keys: [
    { pubkey: playerPk, isSigner: true, isWritable: true },
    { pubkey: userVaultPk, isSigner: false, isWritable: true },
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ],
  data: Buffer.concat([disc("deposit_user"), Buffer.from(new BN(amount).toArray("le", 8))]),
});

// NEW: deposit_admin(amount)
const ixDepositAdmin = ({ adminPk, adminConfigPk, vaultPk, amount }) => ({
  programId: PROGRAM_ID,
  keys: [
    { pubkey: adminPk, isSigner: true, isWritable: true },
    { pubkey: adminConfigPk, isSigner: false, isWritable: false },
    { pubkey: vaultPk, isSigner: false, isWritable: true },
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ],
  data: Buffer.concat([disc("deposit_admin"), Buffer.from(new BN(amount).toArray("le", 8))]),
});

const ixLock = ({
  playerPk,
  userVaultPk,
  vaultPk,
  pendingPk,
  payerPk, // relayer/admin
  entryLamports,
  side,
  nonce,
  expiryUnix,
}) => ({
  programId: PROGRAM_ID,
  keys: [
    { pubkey: playerPk, isSigner: false, isWritable: false },
    { pubkey: userVaultPk, isSigner: false, isWritable: true },
    { pubkey: vaultPk, isSigner: false, isWritable: true },
    { pubkey: pendingPk, isSigner: false, isWritable: true },
    { pubkey: payerPk, isSigner: true, isWritable: true },
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ],
  data: Buffer.concat([
    disc("lock"),
    Buffer.from(new BN(entryLamports).toArray("le", 8)),
    Buffer.from([side]),
    Buffer.from(new BN(nonce).toArray("le", 8)),
    Buffer.from(new BN(expiryUnix).toArray("le", 8)),
  ]),
});

const ixResolve = ({
  playerPk,
  userVaultPk,
  vaultPk,
  adminPk,
  pendingPk,
  checksum,
  payout,
  ed25519IxIndex,
  winnerSide,
}) => ({
  programId: PROGRAM_ID,
  keys: [
    { pubkey: playerPk, isSigner: false, isWritable: false },
    { pubkey: userVaultPk, isSigner: false, isWritable: true },
    { pubkey: vaultPk, isSigner: false, isWritable: true },
    { pubkey: adminPk, isSigner: false, isWritable: false },
    { pubkey: pendingPk, isSigner: false, isWritable: true },
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    { pubkey: SYSVAR_INSTRUCTIONS, isSigner: false, isWritable: false },
  ],
  data: Buffer.concat([
    disc("resolve"),
    Buffer.from([checksum]),
    Buffer.from(new BN(payout).toArray("le", 8)),
    Buffer.from([ed25519IxIndex]),
    Buffer.from([winnerSide]),
  ]),
});

const ixWithdrawUser = ({ playerPk, userVaultPk, payerPk, amount, ed25519IxIndex }) => ({
  programId: PROGRAM_ID,
  keys: [
    { pubkey: playerPk, isSigner: true, isWritable: true },
    { pubkey: userVaultPk, isSigner: false, isWritable: true },
    { pubkey: payerPk, isSigner: true, isWritable: true }, // you can pass playerPk here
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ],
  data: Buffer.concat([
    disc("withdraw_user"),
    Buffer.from(new BN(amount).toArray("le", 8)),
    Buffer.from([ed25519IxIndex]),
  ]),
});

const ixAdminSweep = ({ adminPk, adminConfigPk, vaultPk, toPk, amount }) => ({
  programId: PROGRAM_ID,
  keys: [
    { pubkey: adminPk, isSigner: true, isWritable: false },
    { pubkey: adminConfigPk, isSigner: false, isWritable: false },
    { pubkey: vaultPk, isSigner: false, isWritable: true },
    { pubkey: toPk, isSigner: false, isWritable: true },
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ],
  data: Buffer.concat([disc("admin_sweep"), Buffer.from(new BN(amount).toArray("le", 8))]),
});

/** ---- MAIN ---- */
(async () => {
  const connection = new Connection(RPC_URL, "confirmed");

  console.log("RPC       :", RPC_URL);
  console.log("Program ID:", PROGRAM_ID.toBase58());
  console.log("Admin     :", admin.publicKey.toBase58());
  console.log("User      :", user.publicKey.toBase58());

  const vaultPda = pdaVault();
  const adminConfigPda = pdaAdminConfig();
  const userVaultPda = pdaUserVault(user.publicKey);

  console.log("Vault PDA :", vaultPda.toBase58());
  console.log("Admin PDA :", adminConfigPda.toBase58());
  console.log("User Vault:", userVaultPda.toBase58());

  const adminBytes = Array.from(admin.publicKey.toBytes());

  // ---------------- SETTINGS ----------------
  const ENTRY = 1e8;       // 0.1 SOL bet
  const NONCE = 1;
  const EXPIRY = Math.floor(Date.now() / 1000) + 60;
  const PAYOUT = 2e8;      // 0.2 SOL payout (demo)
  const SIDE = 0;          // 0 or 1
  const CHECKSUM = ((NONCE % 251) + 1) & 0xff;

  const WANT_USER_VAULT_BAL = ENTRY * 3; // top-up target
  const WANT_VAULT_BAL = PAYOUT * 2;     // house liquidity target

  // ------------- Build instructions (skip if exist) -------------
  const ixs = [];
  ixs.push(ComputeBudgetProgram.setComputeUnitPrice({ microLamports: 1 }));
  ixs.push(ComputeBudgetProgram.setComputeUnitLimit({ units: 200_000 }));

  if (!(await connection.getAccountInfo(adminConfigPda))) {
    console.log("➡️  init_admin");
    ixs.push(ixInitAdmin({ authorityPk: admin.publicKey, adminConfigPk: adminConfigPda, adminBytes }));
  }
  if (!(await connection.getAccountInfo(vaultPda))) {
    console.log("➡️  initialize vault");
    ixs.push(ixInitialize({ authorityPk: admin.publicKey, vaultPk: vaultPda }));
  }
  if (!(await connection.getAccountInfo(userVaultPda))) {
    console.log("➡️  init_user_vault");
    ixs.push(ixInitUserVault({ playerPk: user.publicKey, userVaultPk: userVaultPda }));
  }

  // deposits (only if needed)
  const uvBal = await connection.getBalance(userVaultPda);
  if (uvBal < WANT_USER_VAULT_BAL) {
    const need = WANT_USER_VAULT_BAL - uvBal;
    console.log(`➡️  deposit_user ${need} lamports`);
    ixs.push(ixDepositUser({ playerPk: user.publicKey, userVaultPk: userVaultPda, amount: need }));
  }

  const vaultBal = await connection.getBalance(vaultPda);
  if (vaultBal < WANT_VAULT_BAL) {
    const need = WANT_VAULT_BAL - vaultBal;
    console.log(`➡️  deposit_admin ${need} lamports`);
    ixs.push(ixDepositAdmin({ adminPk: admin.publicKey, adminConfigPk: adminConfigPda, vaultPk: vaultPda, amount: need }));
  }

  // game flow (admin pays rent/fees; **no user signature needed** after deposit)
  const pendingPda = pdaPending(user.publicKey, NONCE);
  ixs.push(
    ixLock({
      playerPk: user.publicKey,
      userVaultPk: userVaultPda,
      vaultPk: vaultPda,
      pendingPk: pendingPda,
      payerPk: admin.publicKey, // relayer/admin pays
      entryLamports: ENTRY,
      side: SIDE,
      nonce: NONCE,
      expiryUnix: EXPIRY,
    })
  );
  ixs.push(
    ixResolve({
      playerPk: user.publicKey,
      userVaultPk: userVaultPda,
      vaultPk: vaultPda,
      adminPk: admin.publicKey,
      pendingPk: pendingPda,
      checksum: CHECKSUM,
      payout: PAYOUT,
      ed25519IxIndex: 0,
      winnerSide: SIDE,
    })
  );

  // optional: user withdraw a bit back to their wallet (requires user signature)
  ixs.push(
    ixWithdrawUser({
      playerPk: user.publicKey,
      userVaultPk: userVaultPda,
      payerPk: user.publicKey, // just pass the same signer
      amount: ENTRY,
      ed25519IxIndex: 0,
    })
  );

  // optional: admin sweep profits
  ixs.push(
    ixAdminSweep({
      adminPk: admin.publicKey,
      adminConfigPk: adminConfigPda,
      vaultPk: vaultPda,
      toPk: admin.publicKey,
      amount: 1e7, // 0.01 SOL
    })
  );

  // ------------- Send -------------
  const { blockhash } = await connection.getLatestBlockhash("confirmed");
  const msg = new TransactionMessage({
    // Set the fee payer to admin so gameplay runs without user popup.
    // (User still signs deposit/withdraw steps included in this tx.)
    payerKey: admin.publicKey,
    recentBlockhash: blockhash,
    instructions: ixs,
  }).compileToV0Message();

  const vtx = new VersionedTransaction(msg);
  vtx.sign([admin, user]); // both present if deposit/withdraw included

  try {
    const sig = await connection.sendRawTransaction(vtx.serialize(), {
      skipPreflight: false,
      maxRetries: 3,
    });
    await connection.confirmTransaction(sig, "confirmed");
    console.log("✅ Full flow tx:", sig);
  } catch (e) {
    console.error("Flow failed:", e?.message || e);
    try {
      const sim = await connection.simulateTransaction(vtx, {
        sigVerify: false,
        replaceRecentBlockhash: true,
      });
      if (sim?.value?.logs) {
        console.error("Simulation logs:");
        for (const l of sim.value.logs) console.error(l);
      }
    } catch (_) {}
  }
})();
