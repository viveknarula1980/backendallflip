// tests/init_mines_manual.js
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

if (!process.env.MINES_PROGRAM_ID) throw new Error("Set MINES_PROGRAM_ID=<Mines program id>");
if (!process.env.ANCHOR_WALLET) throw new Error("Set ANCHOR_WALLET=~/.config/solana/id.json");

const PROGRAM_ID = new PublicKey(process.env.MINES_PROGRAM_ID);

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
function buildInitializeIx({ payerPk, vaultPk }) {
  // Assuming your instruction is named `initialize` in Rust
  const data = anchorDisc("initialize"); // no args
  const keys = [
    { pubkey: payerPk, isSigner: true, isWritable: true }, // authority
    { pubkey: vaultPk, isSigner: false, isWritable: true }, // vault PDA
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
  console.log("Vault PDA :", vaultPda.toBase58());

  const ixInit = buildInitializeIx({
    payerPk: payer.publicKey,
    vaultPk: vaultPda,
  });

  // CU headroom (optional)
  const cuPrice = ComputeBudgetProgram.setComputeUnitPrice({ microLamports: 1 });
  const cuLimit = ComputeBudgetProgram.setComputeUnitLimit({ units: 200_000 });

  const { blockhash } = await connection.getLatestBlockhash("confirmed");
  const msg = new TransactionMessage({
    payerKey: payer.publicKey,
    recentBlockhash: blockhash,
    instructions: [cuPrice, cuLimit, ixInit],
  }).compileToV0Message();

  const vtx = new VersionedTransaction(msg);
  vtx.sign([payer]);

  try {
    const sig = await connection.sendRawTransaction(vtx.serialize(), {
      skipPreflight: false,
      maxRetries: 3,
    });
    await connection.confirmTransaction(sig, "confirmed");
    console.log("✅ initialize_vault:", sig);
  } catch (e) {
    console.error("init_vault failed:", e?.message || e);
  }

  console.log("Done.");
})();
