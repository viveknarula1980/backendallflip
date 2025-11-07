const anchor = require("@coral-xyz/anchor");
const { PublicKey, SystemProgram } = anchor.web3;
const fs = require("fs");
const path = require("path");

(async () => {
  try {
    // --- provider + program ---
    const provider = anchor.AnchorProvider.env();
    anchor.setProvider(provider);
    const program = anchor.workspace.Casino;

    console.log("Program ID:", program.programId.toBase58());

    // --- load admin keypair from ANCHOR_WALLET (or default id.json) ---
    const walletPath =
      process.env.ANCHOR_WALLET ||
      path.join(process.env.HOME, ".config", "solana", "id.json");

    const secret = JSON.parse(fs.readFileSync(walletPath, "utf8"));
    const adminKeypair = anchor.web3.Keypair.fromSecretKey(
      Uint8Array.from(secret)
    );

    console.log("Admin keypair pubkey:", adminKeypair.publicKey.toBase58());
    console.log("Provider wallet pubkey:", provider.wallet.publicKey.toBase58());

    // --- derive house vault PDA: [b"vault"] ---
    const [houseVault] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault")],
      program.programId
    );

    console.log("House vault PDA:", houseVault.toBase58());

    // --- destination (where you want to receive SOL) ---
    // you can change this to any address you want
    const destination = adminKeypair.publicKey;
    console.log("Destination:", destination.toBase58());

    // --- check house vault balance first (optional but useful) ---
    const hvBalance = await provider.connection.getBalance(houseVault);
    console.log("House vault balance (lamports):", hvBalance);

    // amount to withdraw (lamports)
    const amountLamports = 0.2 * 1e9; // 0.2 SOL
    if (hvBalance < amountLamports) {
      console.error(
        `House vault has only ${hvBalance} lamports, need ${amountLamports}.`
      );
      return;
    }

    const amount = new anchor.BN(amountLamports);

    // --- call house_withdraw ---
    const txSig = await program.methods
      .houseWithdraw({ amount })
      .accounts({
        admin: adminKeypair.publicKey,
        houseVault,
        destination,
        systemProgram: SystemProgram.programId,
      })
      .signers([adminKeypair]) // IMPORTANT: admin signs
      .rpc();

    console.log("✅ house_withdraw success, tx:", txSig);
  } catch (err) {
    console.error("❌ house_withdraw failed:");
    console.error(err);
  }
})();
