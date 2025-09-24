const fs = require("fs");
const { Connection, Keypair, VersionedTransaction, TransactionMessage, SystemProgram, PublicKey } = require("@solana/web3.js");

(async () => {
  const connection = new Connection("https://api.devnet.solana.com", "confirmed");

  // Load local keypair
  const secret = Uint8Array.from(JSON.parse(fs.readFileSync(process.env.HOME + "/.config/solana/id.json")));
  const kp = Keypair.fromSecretKey(secret);

  // Load base64 transaction
  const txBase64 = fs.readFileSync("tx.b64", "utf8").trim();
  const tx = VersionedTransaction.deserialize(Buffer.from(txBase64, "base64"));

  // Sign with local keypair
  tx.sign([kp]);

  // Send
  const sig = await connection.sendTransaction(tx);
  console.log("Transaction signature:", sig);

  const confirm = await connection.confirmTransaction(sig, "confirmed");
  console.log("Confirmed:", confirm);
})();
