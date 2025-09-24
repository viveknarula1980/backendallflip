// tests/e2e_real_devnet.js
require("dotenv").config();
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const bs58 = require("bs58");
const nacl = require("tweetnacl");
const {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
  ComputeBudgetProgram,
  Ed25519Program,
  sendAndConfirmTransaction,
} = require("@solana/web3.js");

// ---------- Config ----------
const RPC =
  process.env.ANCHOR_PROVIDER_URL ||
  process.env.CLUSTER ||
  "https://api.devnet.solana.com";
const WALLET = process.env.ANCHOR_WALLET || path.join(process.env.HOME, ".config/solana/id.json");
const PROGRAM_ID = new PublicKey(process.env.PROGRAM_ID);

// Bet params (override with env if you want)
const BET_AMOUNT_LAMPORTS = Number(process.env.BET_AMOUNT_LAMPORTS || 1_000_000); // 0.001 SOL
const BET_TYPE_NUM = Number(process.env.BET_TYPE_NUM || 0); // 0=under, 1=over
const TARGET_NUMBER = Number(process.env.TARGET_NUMBER || 55); // 2..98
const RTP_BPS = Number(process.env.RTP_BPS || 9900); // 99.00%
const EXPIRY_SECS = Number(process.env.NONCE_TTL_SECONDS || 300);

// ---------- Helpers ----------
const disc = (name) => crypto.createHash("sha256").update(`global:${name}`).digest().slice(0, 8);
const u64le = (n) => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };
const i64le = (n) => { const b = Buffer.alloc(8); b.writeBigInt64LE(BigInt(n)); return b; };
const nonceBufLE = (n) => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };
const loadKp = (p) => Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync(p, "utf8"))));

function canonicalMessage({ programId, vault, player, betAmount, betType, target, roll, payout, nonce, expiryUnix }) {
  const parts = [];
  parts.push(Buffer.from("DICE_V1"));
  parts.push(Buffer.from(programId)); // 32
  parts.push(Buffer.from(vault));     // 32
  parts.push(Buffer.from(player));    // 32
  parts.push(u64le(betAmount));
  parts.push(Buffer.from([betType & 0xff]));
  parts.push(Buffer.from([target & 0xff]));
  parts.push(Buffer.from([roll & 0xff]));
  parts.push(u64le(payout));
  parts.push(u64le(nonce));
  parts.push(i64le(expiryUnix));
  return Buffer.concat(parts);
}

function adminSecret64FromEnv() {
  const b = bs58.decode(process.env.ADMIN_PRIVKEY_BASE58 || "");
  if (b.length === 64) return Uint8Array.from(b);
  if (b.length === 32) return nacl.sign.keyPair.fromSeed(Uint8Array.from(b)).secretKey;
  throw new Error("ADMIN_PRIVKEY_BASE58 must decode to 32 or 64 bytes");
}

async function airdropIfNeeded(conn, pubkey, minLamports) {
  const bal = await conn.getBalance(pubkey, "confirmed");
  if (bal >= minLamports) return;
  console.log(`Airdropping 1 SOL to ${pubkey.toBase58()} (balance was ${bal})…`);
  const sig = await conn.requestAirdrop(pubkey, 1_000_000_000);
  await conn.confirmTransaction(sig, "confirmed");
}

async function sendTx(conn, kp, instructions, label) {
  const tx = new Transaction();
  tx.add(ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }), ...instructions);
  tx.feePayer = kp.publicKey;
  const sig = await sendAndConfirmTransaction(conn, tx, [kp], { commitment: "confirmed" });
  console.log(`✅ ${label}: ${sig}  (https://explorer.solana.com/tx/${sig}?cluster=devnet)`);
  return sig;
}

(async () => {
  if (!process.env.PROGRAM_ID) throw new Error("PROGRAM_ID missing");
  if (!process.env.ADMIN_PUBKEY_BASE58) throw new Error("ADMIN_PUBKEY_BASE58 missing");
  if (!process.env.ADMIN_PRIVKEY_BASE58) throw new Error("ADMIN_PRIVKEY_BASE58 missing");

  const conn = new Connection(RPC, "confirmed");
  const player = loadKp(WALLET);
  await airdropIfNeeded(conn, player.publicKey, BET_AMOUNT_LAMPORTS + 100_000); // bet + fees

  const [vault] = PublicKey.findProgramAddressSync([Buffer.from("vault")], PROGRAM_ID);
  const [adminPda] = PublicKey.findProgramAddressSync([Buffer.from("admin")], PROGRAM_ID);

  console.log("Program ID :", PROGRAM_ID.toBase58());
  console.log("Player     :", player.publicKey.toBase58());
  console.log("Vault PDA  :", vault.toBase58());
  console.log("Admin PDA  :", adminPda.toBase58());

  // --- Ensure PDAs are initialized (init_vault + init_admin) ---
  const ixVault = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: player.publicKey, isSigner: true, isWritable: true },
      { pubkey: vault, isSigner: false, isWritable: true },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: disc("init_vault"),
  });

  const adminPk = Buffer.from(bs58.decode(process.env.ADMIN_PUBKEY_BASE58));
  const ixAdmin = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: player.publicKey, isSigner: true, isWritable: true }, // authority
      { pubkey: adminPda, isSigner: false, isWritable: true },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: Buffer.concat([disc("init_admin"), adminPk]),
  });

  // try combined, then fall back silently
  try { await sendTx(conn, player, [ixVault, ixAdmin], "init_vault + init_admin"); }
  catch (e) {
    const s = String(e.message || e);
    if (!s.includes("already in use")) {
      try { await sendTx(conn, player, [ixVault], "init_vault"); } catch {}
      try { await sendTx(conn, player, [ixAdmin], "init_admin"); } catch {}
    }
  }

  // --- Step 1: place_bet_lock ---
  if (TARGET_NUMBER < 2 || TARGET_NUMBER > 98) throw new Error("TARGET_NUMBER must be 2..98");
  if (BET_TYPE_NUM !== 0 && BET_TYPE_NUM !== 1) throw new Error("BET_TYPE_NUM must be 0 or 1");

  const nonce = BigInt(Date.now());
  const expiryUnix = Math.floor(Date.now() / 1000) + EXPIRY_SECS;

  const pendingBetPda = PublicKey.findProgramAddressSync(
    [Buffer.from("bet"), player.publicKey.toBuffer(), nonceBufLE(nonce)],
    PROGRAM_ID
  )[0];

  const preBal = await conn.getBalance(player.publicKey, "confirmed");

  const dataLock = Buffer.concat([
    disc("place_bet_lock"),
    u64le(BET_AMOUNT_LAMPORTS),
    Buffer.from([BET_TYPE_NUM & 0xff]),
    Buffer.from([TARGET_NUMBER & 0xff]),
    u64le(nonce),
    i64le(expiryUnix),
  ]);

  const ixLock = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: player.publicKey, isSigner: true, isWritable: true },
      { pubkey: vault, isSigner: false, isWritable: true },
      { pubkey: pendingBetPda, isSigner: false, isWritable: true },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: dataLock,
  });

  await sendTx(conn, player, [ixLock], "place_bet_lock");

  // --- Step 2: resolve_bet (with ed25519 pre-instruction signed by backend) ---
  const roll = crypto.randomInt(1, 101);
  const odds = BET_TYPE_NUM === 0 ? (TARGET_NUMBER - 1) : (100 - TARGET_NUMBER);
  const win = BET_TYPE_NUM === 0 ? (roll < TARGET_NUMBER) : (roll > TARGET_NUMBER);
  const payout = win ? Number((BigInt(BET_AMOUNT_LAMPORTS) * BigInt(RTP_BPS)) / (100n * BigInt(odds))) : 0;

  const message = canonicalMessage({
    programId: PROGRAM_ID.toBuffer(),
    vault: vault.toBuffer(),
    player: player.publicKey.toBuffer(),
    betAmount: BET_AMOUNT_LAMPORTS,
    betType: BET_TYPE_NUM,
    target: TARGET_NUMBER,
    roll,
    payout,
    nonce: Number(nonce),
    expiryUnix,
  });

  const adminSecret64 = adminSecret64FromEnv();
  const adminSig = nacl.sign.detached(Uint8Array.from(message), adminSecret64);
  const adminPub = adminSecret64.slice(32);

  const edIx = Ed25519Program.createInstructionWithPublicKey({
    publicKey: Buffer.from(adminPub),
    message: Buffer.from(message),
    signature: Buffer.from(adminSig),
  });
  const edIndex = 1; // we put ComputeBudget at [0], ed25519 at [1]

  const SYSVAR_INSTRUCTIONS = new PublicKey("Sysvar1nstructions1111111111111111111111111");
  const dataResolve = Buffer.concat([
    disc("resolve_bet"),
    Buffer.from([roll & 0xff]),
    u64le(payout),
    Buffer.from([edIndex & 0xff]),
  ]);

  const ixResolve = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: player.publicKey, isSigner: false, isWritable: true },
      { pubkey: vault, isSigner: false, isWritable: true },
      { pubkey: adminPda, isSigner: false, isWritable: false },
      { pubkey: pendingBetPda, isSigner: false, isWritable: true },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
      { pubkey: SYSVAR_INSTRUCTIONS, isSigner: false, isWritable: false },
    ],
    data: dataResolve,
  });

  await sendTx(conn, player, [edIx, ixResolve], "resolve_bet");

  const postBal = await conn.getBalance(player.publicKey, "confirmed");
  console.log(`\n=== RESULT ===
roll=${roll}  win=${win}  payout=${payout}
player balance Δ = ${postBal - preBal} lamports (pre=${preBal}, post=${postBal})
pending_bet PDA = ${pendingBetPda.toBase58()}
`);
})().catch((e) => {
  console.error("E2E failed:", e);
  process.exit(1);
});
