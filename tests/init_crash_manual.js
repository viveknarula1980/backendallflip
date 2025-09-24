// tests/init_crash_manual.js
require("dotenv").config();

const fs = require("fs");
const os = require("os");
const path = require("path");
const bs58 = require("bs58");
const crypto = require("crypto");
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

if (!process.env.PROGRAM_ID) throw new Error("Set PROGRAM_ID=<Crash program id>");
if (!process.env.ANCHOR_WALLET) throw new Error("Set ANCHOR_WALLET=~/.config/solana/id.json");
if (!process.env.ADMIN_PUBKEY_BASE58) throw new Error("Set ADMIN_PUBKEY_BASE58=<ed25519 pubkey (32B) base58>");

const PROGRAM_ID = new PublicKey(process.env.PROGRAM_ID);

/** ---- Load payer ---- */
function loadKeypair(filePath) {
  const abs = filePath.startsWith("~")
    ? path.join(os.homedir(), filePath.slice(1))
    : filePath;
  const secret = JSON.parse(fs.readFileSync(abs, "utf8"));
  return Keypair.fromSecretKey(Uint8Array.from(secret));
}
const payer = loadKeypair(process.env.ANCHOR_WALLET);

/** ---- PDA helpers ---- */
function deriveVaultPda() {
  return PublicKey.findProgramAddressSync([Buffer.from("vault")], PROGRAM_ID)[0];
}
function deriveAdminPda() {
  return PublicKey.findProgramAddressSync([Buffer.from("admin")], PROGRAM_ID)[0];
}

/** ---- Anchor discriminator helpers ---- */
function anchorDisc(ixNameSnakeCase) {
  // 8 bytes: sha256("global:<name>")
  return crypto
    .createHash("sha256")
    .update(`global:${ixNameSnakeCase}`)
    .digest()
    .slice(0, 8);
}

/** ---- Build raw instructions ---- */
function buildInitVaultIx({ payerPk, vaultPk }) {
  const data = anchorDisc("init_vault"); // no args
  const keys = [
    { pubkey: payerPk, isSigner: true, isWritable: true }, // payer
    { pubkey: vaultPk, isSigner: false, isWritable: true }, // vault PDA (created inside program)
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ];
  return { programId: PROGRAM_ID, keys, data };
}

function buildInitAdminIx({ authorityPk, adminPda, adminPubkey32 }) {
  const disc = anchorDisc("init_admin");
  if (adminPubkey32.length !== 32) throw new Error("admin pubkey must be 32 bytes");
  const data = Buffer.concat([disc, Buffer.from(adminPubkey32)]);
  const keys = [
    { pubkey: authorityPk, isSigner: true, isWritable: true },  // authority
    { pubkey: adminPda,   isSigner: false, isWritable: true },  // admin_config PDA
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ];
  return { programId: PROGRAM_ID, keys, data };
}

(async () => {
  const connection = new Connection(RPC_URL, "confirmed");

  console.log("RPC       :", RPC_URL);
  console.log("Program ID:", PROGRAM_ID.toBase58());
  console.log("Payer     :", payer.publicKey.toBase58());

  const vaultPda = deriveVaultPda();
  const adminPda = deriveAdminPda();
  console.log("Vault PDA :", vaultPda.toBase58());
  console.log("Admin PDA :", adminPda.toBase58());

  const adminPubkey32 = bs58.decode(process.env.ADMIN_PUBKEY_BASE58);
  if (adminPubkey32.length !== 32) throw new Error("ADMIN_PUBKEY_BASE58 must decode to 32 bytes");

  const ixVault = buildInitVaultIx({ payerPk: payer.publicKey, vaultPk: vaultPda });
  const ixAdmin = buildInitAdminIx({
    authorityPk: payer.publicKey,
    adminPda,
    adminPubkey32,
  });

  // CU headroom (optional)
  const cuPrice = ComputeBudgetProgram.setComputeUnitPrice({ microLamports: 1 });
  const cuLimit = ComputeBudgetProgram.setComputeUnitLimit({ units: 200_000 });

  const { blockhash } = await connection.getLatestBlockhash("confirmed");
  const msg = new TransactionMessage({
    payerKey: payer.publicKey,
    recentBlockhash: blockhash,
    instructions: [cuPrice, cuLimit, ixVault, ixAdmin],
  }).compileToV0Message();

  const vtx = new VersionedTransaction(msg);
  vtx.sign([payer]);

  let sig;
  try {
    sig = await connection.sendRawTransaction(vtx.serialize(), { skipPreflight: false, maxRetries: 3 });
    await connection.confirmTransaction(sig, "confirmed");
    console.log("✅ init_vault + init_admin:", sig);
  } catch (e) {
    console.warn("Combined failed:", e?.message || e);
    // Try stepwise (ignore "already in use")
    try {
      const { blockhash: bh1 } = await connection.getLatestBlockhash("confirmed");
      const msg1 = new TransactionMessage({
        payerKey: payer.publicKey,
        recentBlockhash: bh1,
        instructions: [cuLimit, ixVault],
      }).compileToV0Message();
      const tx1 = new VersionedTransaction(msg1);
      tx1.sign([payer]);
      const sig1 = await connection.sendRawTransaction(tx1.serialize(), { skipPreflight: false });
      await connection.confirmTransaction(sig1, "confirmed");
      console.log("init_vault:", sig1);
    } catch (e1) {
      console.warn("init_vault skipped/failed:", e1?.message || e1);
    }

    try {
      const { blockhash: bh2 } = await connection.getLatestBlockhash("confirmed");
      const msg2 = new TransactionMessage({
        payerKey: payer.publicKey,
        recentBlockhash: bh2,
        instructions: [cuLimit, ixAdmin],
      }).compileToV0Message();
      const tx2 = new VersionedTransaction(msg2);
      tx2.sign([payer]);
      const sig2 = await connection.sendRawTransaction(tx2.serialize(), { skipPreflight: false });
      await connection.confirmTransaction(sig2, "confirmed");
      console.log("init_admin:", sig2);
    } catch (e2) {
      console.warn("init_admin skipped/failed:", e2?.message || e2);
    }
  }

  console.log("Done.");
})();
