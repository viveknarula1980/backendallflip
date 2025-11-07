// tests/send_local_bet.js
const fs = require("fs");
const path = require("path");
const { Connection, Keypair, VersionedTransaction } = require("@solana/web3.js");

const RPC_URL = "https://api.devnet.solana.com";
const BACKEND_URL = "https://backendgame-1c3u.onrender.com/bets/prepare";
const WALLET_PATH = path.join(process.env.HOME, ".config/solana/id.json");
const BET_AMOUNT_LAMPORTS = 1_000_000; 
const BET_TYPE = "under";               
const TARGET_NUMBER = 10;             

(async () => {
  const connection = new Connection(RPC_URL, "confirmed");

  const secret = Uint8Array.from(JSON.parse(fs.readFileSync(WALLET_PATH, "utf8")));
  const kp = Keypair.fromSecretKey(secret);
  const player = kp.publicKey.toBase58();
  console.log("Player:", player);

  const res = await fetch(BACKEND_URL, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      player,
      betAmountLamports: BET_AMOUNT_LAMPORTS,
      betType: BET_TYPE,
      targetNumber: TARGET_NUMBER,
    }),
  });

  if (!res.ok) {
    const txt = await res.text();
    throw new Error(`Backend error ${res.status}: ${txt}`);
  }
  const data = await res.json();
  if (!data.transactionBase64) {
    console.log("Backend response:", data);
    throw new Error("No transactionBase64 in response");
  }

  console.log(`roll=${data.roll} win=${data.win} payout=${data.payoutLamports}`);
  const txBase64 = String(data.transactionBase64).trim().replace(/\s+/g, "");
  const tx = VersionedTransaction.deserialize(Buffer.from(txBase64, "base64"));
  tx.sign([kp]);

  const sig = await connection.sendTransaction(tx, { skipPreflight: false });
  console.log("Sent tx:", sig);

  const conf = await connection.confirmTransaction(sig, "confirmed");
  console.log("Confirmed:", conf.value.err ? conf.value.err : "OK");
})().catch((e) => {
  console.error("FAILED:", e);
  process.exit(1);
});
