// tests/init_plinko.js (idempotent)
require("dotenv").config();
const fs = require("fs");
const os = require("os");
const path = require("path");
const bs58 = require("bs58");
const crypto = require("crypto");
const {
  Connection, Keypair, PublicKey, SystemProgram,
  Transaction, sendAndConfirmTransaction
} = require("@solana/web3.js");

const RPC_URL = process.env.ANCHOR_PROVIDER_URL || process.env.CLUSTER || "https://api.devnet.solana.com";
if (!process.env.PLINKO_PROGRAM_ID) throw new Error("PLINKO_PROGRAM_ID missing in env");
if (!process.env.ANCHOR_WALLET) throw new Error("ANCHOR_WALLET missing in env");
if (!process.env.ADMIN_PUBKEY_BASE58) throw new Error("ADMIN_PUBKEY_BASE58 missing in env");

const PROGRAM_ID = new PublicKey(process.env.PLINKO_PROGRAM_ID);

function disc(name) {
  return crypto.createHash("sha256").update(`global:${name}`).digest().slice(0, 8);
}
function expandTilde(p) { return p.startsWith("~") ? path.join(os.homedir(), p.slice(1)) : p; }
function loadFeePayer() {
  const p = expandTilde(process.env.ANCHOR_WALLET.trim());
  const raw = fs.readFileSync(p, "utf8").trim();
  if (raw.startsWith("[")) return Keypair.fromSecretKey(Uint8Array.from(JSON.parse(raw)));
  const b58 = bs58.decode(raw); if (b58.length === 64) return Keypair.fromSecretKey(Uint8Array.from(b58));
  throw new Error("Unrecognized key format in ANCHOR_WALLET");
}

(async () => {
  const connection = new Connection(RPC_URL, "confirmed");
  const payer = loadFeePayer();

  const [adminPda] = PublicKey.findProgramAddressSync([Buffer.from("admin")], PROGRAM_ID);
  const [vaultPda] = PublicKey.findProgramAddressSync([Buffer.from("vault")], PROGRAM_ID);

  console.log("RPC:", RPC_URL);
  console.log("Program:", PROGRAM_ID.toBase58());
  console.log("Admin PDA:", adminPda.toBase58());
  console.log("Vault  PDA:", vaultPda.toBase58());
  console.log("Fee payer:", payer.publicKey.toBase58());

  const adminAcc = await connection.getAccountInfo(adminPda);
  const vaultAcc = await connection.getAccountInfo(vaultPda);

  const ixs = [];

  if (!adminAcc) {
    const adminPubBytes = bs58.decode(process.env.ADMIN_PUBKEY_BASE58);
    if (adminPubBytes.length !== 32) throw new Error("ADMIN_PUBKEY_BASE58 must decode to 32 bytes");
    ixs.push({
      programId: PROGRAM_ID,
      keys: [
        { pubkey: payer.publicKey, isSigner: true, isWritable: true }, // authority
        { pubkey: adminPda,        isSigner: false, isWritable: true }, // admin_config (init)
        { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
      ],
      data: Buffer.concat([disc("init_admin"), Buffer.from(adminPubBytes)]),
    });
    console.log("Will send: init_admin");
  } else {
    console.log("admin_config already exists — skipping init_admin");
  }

  if (!vaultAcc) {
    ixs.push({
      programId: PROGRAM_ID,
      keys: [
        { pubkey: payer.publicKey, isSigner: true, isWritable: true }, // payer
        { pubkey: vaultPda,        isSigner: false, isWritable: true }, // vault (create)
        { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
      ],
      data: Buffer.concat([disc("init_vault")]),
    });
    console.log("Will send: init_vault");
  } else {
    console.log("vault already exists — skipping init_vault");
  }

  if (ixs.length === 0) {
    console.log("Nothing to initialize. All good ✅");
    return;
  }

  const tx = new Transaction().add(...ixs);
  const sig = await sendAndConfirmTransaction(connection, tx, [payer], { commitment: "confirmed" });
  console.log("Init tx:", sig);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
