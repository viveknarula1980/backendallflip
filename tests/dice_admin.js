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

// --- CONFIG ---
const RPC_URL = process.env.RPC_URL || "https://api.devnet.solana.com";
const WALLET_PATH = process.env.WALLET_PATH || path.join(process.env.HOME, ".config/solana/id.json");
const PROGRAM_ID = new PublicKey(process.env.PROGRAM_ID || "2m2qnCreEkuSf1CCZmWvjyBgAWkDYF13quCvsyEkDzGT");
// set this to your backend ADMIN **PUBLIC** key (32 bytes, base58)
const ADMIN_PUBKEY_BASE58 = process.env.ADMIN_PUBKEY_BASE58 || "BLBYKR6jyK7DB9aXUecurbPMiJ8epHMVWPsbkiAVyER";

const IDL_CANDIDATES = [
  path.join(__dirname, "../target/idl/dice.json"),
  path.join(__dirname, "../target/idl/anchor_dice.json"),
];

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
  const payer = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(fs.readFileSync(WALLET_PATH, "utf8")))
  );
  const connection = new Connection(RPC_URL, "confirmed");

  const { idl, path: idlPath } = loadIdlOrNull();
  if (idl) {
    console.log("IDL loaded from:", idlPath);
    console.log("IDL name:", idl.name);
    console.log("IDL instructions:", (idl.instructions || []).map((i) => i.name));
  } else {
    console.warn("IDL not found (will use manual encoding fallback). Searched:", IDL_CANDIDATES);
  }

  const [adminPda] = PublicKey.findProgramAddressSync([Buffer.from("admin")], PROGRAM_ID);
  const adminU8 = anchor.utils.bytes.bs58.decode(ADMIN_PUBKEY_BASE58);
  if (adminU8.length !== 32) {
    throw new Error(`ADMIN_PUBKEY_BASE58 must decode to 32 bytes (PUBLIC key). Got ${adminU8.length}.`);
  }

  // --- Encode: IDL first, else manual ---
  let data;
  if (idl) {
    try {
      const coder = new anchor.BorshCoder(idl);
      const names = (idl.instructions || []).map((i) => i.name);
      let encoded = null;
      for (const nm of ["initAdmin", "init_admin"]) {
        if (names.includes(nm)) {
          encoded = coder.instruction.encode(nm, { adminPubkey: Array.from(adminU8) });
          console.log(`Using coder with instruction name: ${nm}`);
          break;
        }
      }
      if (!encoded) throw new Error("No matching instruction name found in IDL for initAdmin/init_admin");
      data = encoded;
    } catch (e) {
      console.warn("Coder path failed, using MANUAL encoding:", e.message);
    }
  }
  if (!data) {
    const disc = anchorDiscriminator("init_admin");
    data = Buffer.concat([disc, Buffer.from(adminU8)]);
  }

  const keys = [
    { pubkey: payer.publicKey, isSigner: true, isWritable: true }, // authority
    { pubkey: adminPda,        isSigner: false, isWritable: true }, // admin_config
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ];

  const ix = new TransactionInstruction({ programId: PROGRAM_ID, keys, data });
  const tx = new Transaction().add(ix);
  const sig = await sendAndConfirmTransaction(connection, tx, [payer], { commitment: "confirmed" });

  console.log("✅ init_admin tx:", sig);
  console.log("Admin PDA:", adminPda.toBase58());
})().catch((e) => {
  console.error("FAILED:", e);
  console.error("Hints:");
  console.error(" • Make sure IDL exists after `anchor build`");
  console.error(" • ADMIN_PUBKEY_BASE58 must be a 32-byte base58 public key");
  console.error(" • Wallet should have Devnet SOL");
  process.exit(1);
});     