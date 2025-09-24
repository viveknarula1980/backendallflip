require("dotenv").config();
const bs58 = require("bs58");
const {
  Connection, PublicKey, Keypair,
  SystemProgram, Transaction, sendAndConfirmTransaction
} = require("@solana/web3.js");

const RPC = process.env.ANCHOR_PROVIDER_URL || "https://api.devnet.solana.com";
const PROGRAM_ID = new PublicKey(process.env.PLINKO_PROGRAM_ID);
const ADMIN_PUBKEY_BASE58 = process.env.ADMIN_PUBKEY_BASE58;

const SYS = SystemProgram.programId;

function pda(seed) {
  return PublicKey.findProgramAddressSync([Buffer.from(seed)], PROGRAM_ID)[0];
}

(async () => {
  const connection = new Connection(RPC, "confirmed");
  const payer = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(require("fs").readFileSync(process.env.ANCHOR_WALLET || `${require("os").homedir()}/.config/solana/id.json`, "utf8")))
  );

  const vault = pda("vault");
  const admin = pda("admin");

  // ix: init_vault
  const dataLock = (() => {
    const disc = require("crypto").createHash("sha256").update("global:init_vault").digest().slice(0, 8);
    return Buffer.from(disc);
  })();
  const keysVault = [
    { pubkey: payer.publicKey, isSigner: true, isWritable: true },
    { pubkey: vault, isSigner: false, isWritable: true },
    { pubkey: SYS, isSigner: false, isWritable: false },
  ];
  const ixVault = new (require("@solana/web3.js").TransactionInstruction)({
    programId: PROGRAM_ID, keys: keysVault, data: dataLock
  });

  // ix: init_admin([u8;32])
  const dataAdmin = (() => {
    const disc = require("crypto").createHash("sha256").update("global:init_admin").digest().slice(0, 8);
    const adminPk = bs58.decode(ADMIN_PUBKEY_BASE58);
    return Buffer.concat([disc, Buffer.from(adminPk)]);
  })();
  const keysAdmin = [
    { pubkey: payer.publicKey, isSigner: true, isWritable: true },
    { pubkey: admin, isSigner: false, isWritable: true },
    { pubkey: SYS, isSigner: false, isWritable: false },
  ];
  const ixAdmin = new (require("@solana/web3.js").TransactionInstruction)({
    programId: PROGRAM_ID, keys: keysAdmin, data: dataAdmin
  });

  const tx = new Transaction().add(ixVault, ixAdmin);
  const sig = await sendAndConfirmTransaction(connection, tx, [payer]);
  console.log("✅ init_vault + init_admin:", sig);
  console.log("Vault:", vault.toBase58());
  console.log("Admin:", admin.toBase58());
})();
