// tests/init.js — pure web3.js init (no Anchor Program/IDL needed)
require("dotenv").config();
const fs = require("fs");
const path = require("path");
const bs58 = require("bs58");
const crypto = require("crypto");
const {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
  sendAndConfirmTransaction,
} = require("@solana/web3.js");

// 8-byte Anchor discriminator
function anchorDisc(name) {
  return crypto.createHash("sha256").update(`global:${name}`).digest().slice(0, 8);
}

function loadKeypair(fp) {
  const raw = fs.readFileSync(fp, "utf8").trim();
  return Keypair.fromSecretKey(Uint8Array.from(JSON.parse(raw)));
}

(async () => {
  const RPC = process.env.ANCHOR_PROVIDER_URL || "https://api.devnet.solana.com";
  const WALLET = process.env.ANCHOR_WALLET || path.join(process.env.HOME, ".config/solana/id.json");
  const PROGRAM_ID = new PublicKey(process.env.PROGRAM_ID);
  const ADMIN_PUBKEY_BASE58 = process.env.ADMIN_PUBKEY_BASE58;
  if (!ADMIN_PUBKEY_BASE58) throw new Error("ADMIN_PUBKEY_BASE58 missing in env");

  const payer = loadKeypair(WALLET);
  const connection = new Connection(RPC, "confirmed");

  const [vault] = PublicKey.findProgramAddressSync([Buffer.from("vault")], PROGRAM_ID);
  const [adminPda] = PublicKey.findProgramAddressSync([Buffer.from("admin")], PROGRAM_ID);

  console.log("Program ID :", PROGRAM_ID.toBase58());
  console.log("Payer     :", payer.publicKey.toBase58());
  console.log("Vault PDA :", vault.toBase58());
  console.log("Admin PDA :", adminPda.toBase58());

  // ---- init_vault ----
  // accounts: payer (mut, signer), vault (mut, pda "vault"), system_program
  const initVaultIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      { pubkey: vault,          isSigner: false, isWritable: true },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: anchorDisc("init_vault"), // no args
  });

  // ---- init_admin ----
  // args: admin_pubkey [u8;32]
  const adminPk = Buffer.from(bs58.decode(ADMIN_PUBKEY_BASE58));
  if (adminPk.length !== 32) throw new Error("ADMIN_PUBKEY_BASE58 must decode to 32 bytes");
  const initAdminData = Buffer.concat([anchorDisc("init_admin"), adminPk]);

  // accounts: authority (mut, signer), admin_config (mut pda "admin"), system_program
  const initAdminIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: payer.publicKey, isSigner: true, isWritable: true }, // authority
      { pubkey: adminPda,        isSigner: false, isWritable: true }, // admin_config
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: initAdminData,
  });

  // send both in one tx (ok even if vault exists; if it errors as "in use", we try admin alone)
  let sig;
  try {
    const tx = new Transaction().add(initVaultIx, initAdminIx);
    tx.feePayer = payer.publicKey;
    sig = await sendAndConfirmTransaction(connection, tx, [payer], { commitment: "confirmed" });
    console.log("✅ init_vault + init_admin tx:", sig);
  } catch (e) {
    const msg = String(e.message || e);
    console.warn("Combined tx failed, retrying stepwise…", msg);

    // try init_vault alone
    try {
      const tx1 = new Transaction().add(initVaultIx);
      tx1.feePayer = payer.publicKey;
      const s1 = await sendAndConfirmTransaction(connection, tx1, [payer], { commitment: "confirmed" });
      console.log("init_vault tx:", s1);
    } catch (e1) {
      console.warn("init_vault skipped/failed:", String(e1.message || e1));
    }

    // try init_admin alone
    try {
      const tx2 = new Transaction().add(initAdminIx);
      tx2.feePayer = payer.publicKey;
      const s2 = await sendAndConfirmTransaction(connection, tx2, [payer], { commitment: "confirmed" });
      console.log("init_admin tx:", s2);
    } catch (e2) {
      console.warn("init_admin skipped/failed:", String(e2.message || e2));
    }
  }

  console.log("Done.");
})().catch((e) => {
  console.error("Init failed:", e);
  process.exit(1);
});
