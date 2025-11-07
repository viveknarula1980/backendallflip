const anchor = require("@coral-xyz/anchor");
const { PublicKey } = anchor.web3;

(async () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.Casino;

  const newAdminBase58 = process.env.ADMIN_PUBKEY_BASE58;
  if (!newAdminBase58) throw new Error("Missing ADMIN_PUBKEY_BASE58");
  const newAdmin = new PublicKey(newAdminBase58);

  const [adminConfig] = PublicKey.findProgramAddressSync(
    [Buffer.from("admin")],
    program.programId
  );

  console.log("Program ID      :", program.programId.toBase58());
  console.log("AdminConfig PDA :", adminConfig.toBase58());
  console.log("New admin pubkey:", newAdmin.toBase58());
  console.log("Authority wallet:", provider.wallet.publicKey.toBase58());

  const tx = await program.methods
    .initAdmin([...newAdmin.toBytes()]) // use init_admin
    .accounts({
      authority: provider.wallet.publicKey,
      adminConfig,
      systemProgram: anchor.web3.SystemProgram.programId,
    })
    .rpc();

  console.log("✅ Admin updated successfully!");
  console.log("Transaction:", tx);
})();
