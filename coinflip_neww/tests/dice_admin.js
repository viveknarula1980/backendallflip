// scripts/init_admin.js
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

// ------------------ CONFIG ------------------
const RPC_URL = process.env.RPC_URL || "https://api.devnet.solana.com";
const WALLET_PATH = process.env.WALLET_PATH || path.join(process.env.HOME, ".config/solana/id.json");
const PROGRAM_ID = new PublicKey(process.env.PROGRAM_ID || "9VHqxhmwKnTnqgynRs9TWctKkV2kCe7eafMknbMfNfEu");
// admin public key in base58 (MUST match the signing wallet for auth)
const ADMIN_PUBKEY_BASE58 = process.env.ADMIN_PUBKEY_BASE58 || "BLBYKR6jyK7DB9aXUecurbPMiJ8epHMVWPsbkiAVyER";

const IDL_CANDIDATES = [
  path.join(__dirname, "../target/idl/dice.json"),
  path.join(__dirname, "../target/idl/anchor_dice.json"),
];

// ------------------ HELPERS ------------------
function anchorDiscriminator(ixName) {
  const preimage = Buffer.from(`global:${ixName}`);
  return crypto.createHash("sha256").update(preimage).digest().subarray(0, 8);
}

function loadIdlOrNull() {
  for (const p of IDL_CANDIDATES) {
    if (fs.existsSync(p)) {
      try {
        return { idl: JSON.parse(fs.readFileSync(p, "utf8")), path: p };
      } catch (_) {}
    }
  }
  return { idl: null, path: null };
}

// ------------------ MAIN ------------------
(async () => {
  const payer = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(fs.readFileSync(WALLET_PATH, "utf8")))
  );
  const connection = new Connection(RPC_URL, "confirmed");

  const { idl, path: idlPath } = loadIdlOrNull();
  if (idl) {
    console.log("IDL loaded from:", idlPath);
  } else {
    console.warn("IDL not found, fallback to manual encoding.");
  }

  const [adminPda] = PublicKey.findProgramAddressSync([Buffer.from("admin")], PROGRAM_ID);
  const adminU8 = anchor.utils.bytes.bs58.decode(ADMIN_PUBKEY_BASE58);
  if (adminU8.length !== 32) {
    throw new Error(`ADMIN_PUBKEY_BASE58 must be a 32-byte public key. Got length: ${adminU8.length}`);
  }

  // encode data
  let data;
  if (idl) {
    try {
      const coder = new anchor.BorshCoder(idl);
      data = coder.instruction.encode("initAdmin", { adminPubkey: Array.from(adminU8) });
    } catch (e) {
      console.warn("IDL encoding failed, using manual fallback:", e.message);
    }
  }
  if (!data) {
    const disc = anchorDiscriminator("init_admin");
    data = Buffer.concat([disc, Buffer.from(adminU8)]);
  }

  const keys = [
    { pubkey: payer.publicKey, isSigner: true, isWritable: true },
    { pubkey: adminPda, isSigner: false, isWritable: true },
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ];

  const ix = new TransactionInstruction({ programId: PROGRAM_ID, keys, data });
  const tx = new Transaction().add(ix);

  console.log("Sending init_admin...");
  const sig = await sendAndConfirmTransaction(connection, tx, [payer], { commitment: "confirmed" });
  console.log("✅ init_admin tx:", sig);
  console.log("Admin PDA:", adminPda.toBase58());

  const acc = await connection.getAccountInfo(adminPda, "confirmed");
  console.log("Admin PDA owner:", acc?.owner?.toBase58());
  console.log("Admin PDA data length:", acc?.data?.length);
})();
