// tests/init_bot_vault.js
require("dotenv").config();

const {
  VersionedTransaction,
  TransactionMessage,
  TransactionInstruction,
  SystemProgram,
} = require("@solana/web3.js");

// ⬇️ get RPC + PDAs from solana.js
const { connection, PROGRAM_ID, deriveUserVaultPda } = require("../backend/solana");
// ⬇️ get the fee-payer/bot from signer.js
const { getServerKeypair } = require("../backend/signer");

const crypto = require("crypto");

// tiny encoders
const disc = (name) => crypto.createHash("sha256").update(`global:${name}`).digest().slice(0, 8);
const u64  = (n) => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };

// Anchor: activate_user_vault(args: { initial_deposit: u64 })
function ixActivateUserVault({ programId, player, userVault, initialDeposit }) {
  const data = Buffer.concat([disc("activate_user_vault"), u64(initialDeposit)]);
  const keys = [
    { pubkey: player,    isSigner: true,  isWritable: true  }, // payer = player
    { pubkey: userVault, isSigner: false, isWritable: true  },
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ];
  return new TransactionInstruction({ programId, keys, data });
}

async function main() {
  const bot = await getServerKeypair();
  const userVault = deriveUserVaultPda(bot.publicKey);

  // If vault already exists, exit gracefully
  const existing = await connection.getAccountInfo(userVault, "confirmed");
  if (existing) {
    console.log("✅ Bot user_vault already exists:", userVault.toBase58());
    return;
  }

  const initialLamports = BigInt(process.env.BOT_VAULT_INITIAL_LAMPORTS ?? "2000000000"); // 2 SOL by default
  const ix = ixActivateUserVault({
    programId: PROGRAM_ID,
    player: bot.publicKey,
    userVault,
    initialDeposit: initialLamports,
  });

  const { blockhash } = await connection.getLatestBlockhash("confirmed");
  const msg = new TransactionMessage({
    payerKey: bot.publicKey,
    recentBlockhash: blockhash,
    instructions: [ix],
  }).compileToV0Message();

  const tx = new VersionedTransaction(msg);
  tx.sign([bot]);
  const sig = await connection.sendRawTransaction(tx.serialize(), { skipPreflight: false });
  await connection.confirmTransaction(sig, "confirmed");

  console.log("🎉 Bot vault activated:", sig, userVault.toBase58());
}

main()
  .then(() => console.log("Done."))
  .catch((err) => {
    console.error("Init bot vault failed:", err?.message || err);
    process.exit(1);
  });
