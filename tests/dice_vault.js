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
// ✅ your deployed program id:
const PROGRAM_ID = new PublicKey(process.env.PROGRAM_ID || "2m2qnCreEkuSf1CCZmWvjyBgAWkDYF13quCvsyEkDzGT");

// Try both common IDL names (depends on your Cargo/Anchor package name)
const IDL_CANDIDATES = [
  path.join(__dirname, "../target/idl/dice.json"),
  path.join(__dirname, "../target/idl/anchor_dice.json"),
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
  // payer keypair
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

  // derive house vault PDA (seed: "vault")
  const [houseVault] = PublicKey.findProgramAddressSync([Buffer.from("vault")], PROGRAM_ID);

  // --- Encode data: IDL coder first, else manual ---
  let data;
  if (idl) {
    try {
      const coder = new anchor.BorshCoder(idl);
      // support either "initHouseVault" or "init_house_vault" depending on IDL casing
      const names = (idl.instructions || []).map((i) => i.name);
      let encoded = null;
      for (const nm of ["initHouseVault", "init_house_vault"]) {
        if (names.includes(nm)) {
          encoded = coder.instruction.encode(nm, {}); // no args
          console.log(`Using coder with instruction name: ${nm}`);
          break;
        }
      }
      if (!encoded) throw new Error("No matching instruction name found in IDL (initHouseVault/init_house_vault).");
      data = encoded;
    } catch (e) {
      console.warn("Coder path failed, using MANUAL encoding:", e.message);
    }
  }
  if (!data) {
    // manual discriminator only (no args)
    const disc = anchorDiscriminator("init_house_vault");
    data = Buffer.from(disc); // no args to append
  }

  const keys = [
    { pubkey: payer.publicKey, isSigner: true, isWritable: true }, // payer
    { pubkey: houseVault,      isSigner: false, isWritable: true }, // house_vault PDA
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ];

  const ix = new TransactionInstruction({ programId: PROGRAM_ID, keys, data });
  const tx = new Transaction().add(ix);

  console.log("Sending init_house_vault...");
  const sig = await sendAndConfirmTransaction(connection, tx, [payer], {
    commitment: "confirmed",
  });

  console.log("✅ init_house_vault tx:", sig);
  console.log("House Vault PDA:", houseVault.toBase58());

  // (Optional) show account lamports to confirm it exists
  const acc = await connection.getAccountInfo(houseVault, "confirmed");
  if (acc) {
    console.log("House Vault lamports:", acc.lamports);
    console.log("Owner (should be SystemProgram):", acc.owner.toBase58());
  } else {
    console.warn("Warning: house vault not found right after tx (network lag?)");
  }
})().catch((e) => {
  console.error("FAILED:", e);
  console.error("Hints:");
  console.error(" • Ensure IDL file exists after `anchor build` (dice.json or anchor_dice.json)");
  console.error(" • Wallet must have Devnet SOL (try `solana airdrop 1`)");
  console.error(" • PROGRAM_ID must match the deployed program");
  process.exit(1);
});