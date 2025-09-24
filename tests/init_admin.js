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

const RPC_URL = "https://api.devnet.solana.com";
const WALLET_PATH = path.join(process.env.HOME, ".config/solana/id.json");
const PROGRAM_ID = new PublicKey("8XdQXT8TiguCysUPoAzXV611mhdgYKAN6G331CXC81GP"); // your deployed id
const ADMIN_PUBKEY_BASE58 = "BLBYKR6jyK7DB9aXUecurbPMiJ8epHMVWPsbkiAVyER"; // PUBLIC key from gen_admin_keys.js
const LOCAL_IDL_PATH = path.join(__dirname, "../target/idl/anchor_dice.json"); // your IDL file

function anchorDiscriminator(ixName) {
  const preimage = Buffer.from(`global:${ixName}`);
  return crypto.createHash("sha256").update(preimage).digest().subarray(0, 8);
}

(async () => {
  const payer = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(fs.readFileSync(WALLET_PATH, "utf8")))
  );
  const connection = new Connection(RPC_URL, "confirmed");

  if (!fs.existsSync(LOCAL_IDL_PATH)) {
    throw new Error(`IDL not found: ${LOCAL_IDL_PATH}. Did you run "anchor build"?`);
  }
  const idl = JSON.parse(fs.readFileSync(LOCAL_IDL_PATH, "utf8"));
  const instructionNames = (idl.instructions || []).map((i) => i.name);
  console.log("IDL name:", idl.name);
  console.log("IDL instructions:", instructionNames);

  const [adminPda] = PublicKey.findProgramAddressSync([Buffer.from("admin")], PROGRAM_ID);
  const adminU8 = anchor.utils.bytes.bs58.decode(ADMIN_PUBKEY_BASE58);
  if (adminU8.length !== 32) {
    throw new Error(
      `ADMIN_PUBKEY_BASE58 must decode to 32 bytes (PUBLIC key). Got ${adminU8.length}.`
    );
  }

  let data;
  try {
    const coder = new anchor.BorshCoder(idl);
    const tryNames = ["initAdmin", "init_admin"]; 
    let encoded = null;
    for (const nm of tryNames) {
      if (instructionNames.includes(nm)) {
        encoded = coder.instruction.encode(nm, { adminPubkey: Array.from(adminU8) });
        console.log(`Using coder with instruction name: ${nm}`);
        break;
      }
    }
    if (!encoded) throw new Error("No matching instruction name found in IDL for init_admin/initAdmin");
    data = encoded;
  } catch (e) {
    console.warn("Coder path failed, using MANUAL encoding:", e.message);

    const disc = anchorDiscriminator("init_admin");
    data = Buffer.concat([disc, Buffer.from(adminU8)]);
  }

  const keys = [
    { pubkey: payer.publicKey, isSigner: true, isWritable: true }, // authority
    { pubkey: adminPda, isSigner: false, isWritable: true },       // admin_config
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ];

  const ix = new TransactionInstruction({ programId: PROGRAM_ID, keys, data });
  const tx = new Transaction().add(ix);
  const sig = await sendAndConfirmTransaction(connection, tx, [payer], {
    commitment: "confirmed",
  });

  console.log("Init admin tx:", sig);
  console.log("Admin PDA:", adminPda.toBase58());
})().catch((e) => {
  console.error("FAILED:", e);
  console.error("Hints:");
  console.error(" • Confirm IDL lists your init instruction as either initAdmin or init_admin");
  console.error(" • Ensure ADMIN_PUBKEY_BASE58 is the 32-byte PUBLIC key");
  console.error(" • Wallet should have Devnet SOL (solana airdrop 1)");
  process.exit(1);
});
