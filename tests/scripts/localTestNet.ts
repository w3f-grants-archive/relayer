// This a simple script to start two local testnet chains and deploy the contracts on both of them
require('dotenv').config();
import readline from 'readline';
import { ethers } from 'ethers';
import { GanacheAccounts, startGanacheServer } from '../startGanacheServer';
import { SignatureBridge } from '@webb-tools/bridges';
import { SignatureBridge as SignatureBridgeContract } from '@webb-tools/contracts';
import { MintableToken } from '@webb-tools/tokens';
import { fetchComponentsFromFilePaths, getChainIdType } from '@webb-tools/utils';
import publicKeyToAddress from 'ethereum-public-key-to-address';
import path from 'path';
import toml from '@iarna/toml';
import { IAnchorDeposit } from '@webb-tools/interfaces';

// Let's first define a localchain
class LocalChain {
  public readonly endpoint: string;
  private readonly server: any;
  public readonly chainId: number;
  constructor(
    public readonly name: string,
    public readonly evmId: number,
    readonly initalBalances: GanacheAccounts[]
  ) {
    this.endpoint = `http://localhost:${evmId}`;
    this.chainId = getChainIdType(evmId);
    this.server = startGanacheServer(evmId, evmId, initalBalances);
  }

  public provider(): ethers.providers.WebSocketProvider {
    return new ethers.providers.WebSocketProvider(this.endpoint);
  }

  public async stop() {
    this.server.close();
  }

  public async deployToken(
    name: string,
    symbol: string,
    wallet: ethers.Signer
  ): Promise<MintableToken> {
    return MintableToken.createToken(name, symbol, wallet);
  }

  public async deploySignatureBridge(
    otherChain: LocalChain,
    localToken: MintableToken,
    otherToken: MintableToken,
    localWallet: ethers.Wallet,
    otherWallet: ethers.Wallet
  ): Promise<SignatureBridge> {
    localWallet.connect(this.provider());
    otherWallet.connect(otherChain.provider());
    const bridgeInput = {
      anchorInputs: {
        asset: {
          [this.chainId]: [localToken.contract.address],
          [otherChain.chainId]: [otherToken.contract.address],
        },
        anchorSizes: [ethers.utils.parseEther('1')],
      },
      chainIDs: [this.chainId, otherChain.chainId],
    };
    const deployerConfig = {
      wallets: {
        [this.chainId]: localWallet,
        [otherChain.chainId]: otherWallet,
      }
    };
    const governorConfig = {
      [this.chainId]: localWallet,
      [otherChain.chainId]: otherWallet,
    }
    const zkComponents = await fetchComponentsFromFilePaths(
      path.resolve(
        __dirname,
        '../protocol-solidity-fixtures/fixtures/bridge/2/poseidon_bridge_2.wasm'
      ),
      path.resolve(
        __dirname,
        '../protocol-solidity-fixtures/fixtures/bridge/2/witness_calculator.js'
      ),
      path.resolve(
        __dirname,
        '../protocol-solidity-fixtures/fixtures/bridge/2/circuit_final.zkey'
      )
    );

    return SignatureBridge.deployFixedDepositBridge(
      bridgeInput,
      deployerConfig,
      governorConfig,
      zkComponents
    );
  }
}

async function main() {
  const relayerPrivateKey =
    '0x0000000000000000000000000000000000000000000000000000000000000001';
  const senderPrivateKey =
    '0x0000000000000000000000000000000000000000000000000000000000000002';
  const recipient = '0x7Bb1Af8D06495E85DDC1e0c49111C9E0Ab50266E';

  const chainA = new LocalChain('Hermes', 5001, [
    {
      balance: ethers.utils.parseEther('1000').toHexString(),
      secretKey: relayerPrivateKey,
    },
    {
      balance: ethers.utils.parseEther('1000').toHexString(),
      secretKey: senderPrivateKey,
    },
  ]);
  const chainB = new LocalChain('Athena', 5002, [
    {
      balance: ethers.utils.parseEther('1000').toHexString(),
      secretKey: relayerPrivateKey,
    },
    {
      balance: ethers.utils.parseEther('1000').toHexString(),
      secretKey: senderPrivateKey,
    },
  ]);
  const chainAWallet = new ethers.Wallet(relayerPrivateKey, chainA.provider());
  const chainBWallet = new ethers.Wallet(relayerPrivateKey, chainB.provider());

  let chainADeposits: IAnchorDeposit[] = [];
  let chainBDeposits: IAnchorDeposit[] = [];

  // do a random transfer on chainA to a random address
  // se we do have different nonce for that account.
  let tx = await chainAWallet.sendTransaction({
    to: '0x0000000000000000000000000000000000000000',
    value: ethers.utils.parseEther('0.001'),
  });
  await tx.wait();
  // Deploy the token on chainA
  const chainAToken = await chainA.deployToken('ChainA', 'webbA', chainAWallet);
  // Deploy the token on chainB
  const chainBToken = await chainB.deployToken('ChainB', 'webbB', chainBWallet);

  console.log('ChainAToken: ', chainAToken.contract.address);
  console.log('ChainBToken: ', chainBToken.contract.address);

  // athena
  let athenaContracts = {
    evm: {
      athenadkg: {
        contracts: [] as any[],
      },
    },
  };

  // hermes
  let hermesContracts = {
    evm: {
      hermesdkg: {
        contracts: [] as any[],
      },
    },
  };

  // Deploy the signature bridge.
  const signatureBridge = await chainA.deploySignatureBridge(
    chainB,
    chainAToken,
    chainBToken,
    chainAWallet,
    chainBWallet
  );
  // get chainA bridge
  const chainASignatureBridge = signatureBridge.getBridgeSide(chainA.chainId)!;
  // get chainB bridge
  const chainBSignatureBridge = signatureBridge.getBridgeSide(chainB.chainId)!;
  // get the anchor on chainA
  const chainASignatureAnchor = signatureBridge.getAnchor(
    chainA.chainId,
    ethers.utils.parseEther('1')
  )!;
  await chainASignatureAnchor.setSigner(chainAWallet);

  const chainAHandler = await chainASignatureAnchor.getHandler();
  console.log('Chain A Handler address: ', chainAHandler)

  // get the anchor on chainB
  const chainBSignatureAnchor = signatureBridge.getAnchor(
    chainB.chainId,
    ethers.utils.parseEther('1')
  )!;
  await chainBSignatureAnchor.setSigner(chainBWallet);

  const chainBHandler = await chainBSignatureAnchor.getHandler();
  console.log('Chain B Handler address: ', chainBHandler)
  
  // approve token spending
  const webbASignatureTokenAddress = signatureBridge.getWebbTokenAddress(
    chainA.chainId
  )!;
  console.log('webbATokenAddress: ', webbASignatureTokenAddress);

  const webbASignatureToken = await MintableToken.tokenFromAddress(
    webbASignatureTokenAddress,
    chainAWallet
  );
  tx = await webbASignatureToken.approveSpending(
    chainASignatureAnchor.contract.address
  );
  await tx.wait();
  await webbASignatureToken.mintTokens(
    chainAWallet.address,
    ethers.utils.parseEther('1000')
  );

  const webbBSignatureTokenAddress = signatureBridge.getWebbTokenAddress(chainB.chainId)!;
  console.log('webbBTokenAddress: ', webbBSignatureTokenAddress);

  const webbBSignatureToken = await MintableToken.tokenFromAddress(
    webbBSignatureTokenAddress,
    chainBWallet
  );
  tx = await webbBSignatureToken.approveSpending(chainBSignatureAnchor.contract.address);
  await tx.wait();
  await webbBSignatureToken.mintTokens(
    chainBWallet.address,
    ethers.utils.parseEther('1000')
  );

  // push the contracts to athena
  athenaContracts.evm.athenadkg.contracts.push({
    contract: 'SignatureBridge',
    address: chainBSignatureBridge.contract.address,
    'deployed-at': 1,
    'events-watcher': {
      enabled: true,
      'polling-interval': 1000,
    },
  });
  // push the contracts to hermes
  hermesContracts.evm.hermesdkg.contracts.push({
    contract: 'SignatureBridge',
    address: chainASignatureBridge.contract.address,
    'deployed-at': 1,
    'events-watcher': {
      enabled: true,
      'polling-interval': 1000,
    },
  });
  // push the contracts to athena
  athenaContracts.evm.athenadkg.contracts.push({
    contract: 'AnchorOverDKG',
    'dkg-node': 'dkglocal',
    address: chainBSignatureAnchor.contract.address,
    'deployed-at': 1,
    size: 1,
    'events-watcher': {
      enabled: true,
      'polling-interval': 1000,
    },
    'withdraw-fee-percentage': 0,
    'withdraw-gaslimit': '0x350000',
    'linked-anchors': [
      {
        chain: 'hermesdkg',
        address: chainASignatureAnchor.contract.address,
      },
    ],
  });
  // push the contracts to hermes
  hermesContracts.evm.hermesdkg.contracts.push({
    contract: 'AnchorOverDKG',
    'dkg-node': 'dkglocal',
    address: chainASignatureAnchor.contract.address,
    'deployed-at': 1,
    size: 1,
    'events-watcher': {
      enabled: true,
      'polling-interval': 1000,
    },
    'withdraw-fee-percentage': 0,
    'withdraw-gaslimit': '0x350000',
    'linked-anchors': [
      {
        chain: 'athenadkg',
        address: chainBSignatureAnchor.contract.address,
      },
    ],
  });

  console.log(
    'ChainA signature bridge (Hermes): ',
    chainASignatureBridge.contract.address
  );
  console.log(
    'ChainA anchor (Hermes): ',
    chainASignatureAnchor.contract.address
  );
  console.log('ChainA token (Hermes): ', webbASignatureToken.contract.address);
  console.log(' --- --- --- --- --- --- --- --- --- --- --- --- ---');
  console.log(
    'ChainB signature bridge (Athena): ',
    chainBSignatureBridge.contract.address
  );
  console.log(
    'ChainB anchor (Athena): ',
    chainBSignatureAnchor.contract.address
  );
  console.log('ChainB token (Athena): ', webbBSignatureToken.contract.address);
  console.log('\n');
  // print the config for both networks
  console.log('Hermes config:');
  console.log(toml.stringify(hermesContracts));
  console.log('\n');
  console.log('Athena config:');
  console.log(toml.stringify(athenaContracts));
  // stop the server on Ctrl+C or SIGINT singal
  process.on('SIGINT', () => {
    chainA.stop();
    chainB.stop();
  });
  printAvailableCommands();

  // setup readline
  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
  });

  rl.on('line', async (cmdRaw) => {
    const cmd = cmdRaw.trim();
    if (cmd === 'exit') {
      // shutdown the servers
      await chainA.stop();
      await chainB.stop();
      rl.close();
      return;
    }
    // check if cmd is deposit chainA
    if (cmd.startsWith('deposit on chain a')) {
      console.log('Depositing Chain A, please wait...');
      const deposit2 = await chainASignatureAnchor.deposit(chainB.chainId);
      chainADeposits.push(deposit2);
      console.log('Deposit on chain A (signature): ', deposit2);
      // await signatureBridge.updateLinkedAnchors(chainASignatureAnchor);
      return;
    }

    if (cmd.startsWith('deposit on chain b')) {
      console.log('Depositing Chain B, please wait...');
      const deposit2 = await chainBSignatureAnchor.deposit(chainA.chainId);
      chainBDeposits.push(deposit2);
      console.log('Deposit on chain B (signature): ', deposit2);
      // await signatureBridge.updateLinkedAnchors(chainASignatureAnchor);
      return;
    }

    if (cmd.startsWith('withdraw on chain a')) {
      // take a deposit from the chain B
      await signatureBridge.updateLinkedAnchors(chainBSignatureAnchor);

      const result = await signatureBridge.withdraw(
        chainBDeposits.pop()!,
        ethers.utils.parseEther('1'),
        recipient,
        chainAWallet.address,
        chainAWallet
      );
      result ? console.log('withdraw success') : console.log('withdraw failure');
      return;
    }

    if (cmd.startsWith('withdraw on chain b')) {
      // take a deposit from the chain B
      await signatureBridge.updateLinkedAnchors(chainASignatureAnchor);

      let result: boolean = false;
      try {
        result = await signatureBridge.withdraw(
          chainADeposits.pop()!,
          ethers.utils.parseEther('1'),
          recipient,
          chainBWallet.address,
          chainBWallet
        );
      } catch (e) {
        console.log('ERROR: ', e);
      }
      result ? console.log('withdraw success') : console.log('withdraw failure');
      return;
    }

    if (cmd.match(/^spam chain a (\d+)$/)) {
      const txs = parseInt(cmd.match(/^spam chain a (\d+)$/)?.[1] ?? '1');
      console.log(`Spamming Chain A with ${txs} Tx, please wait...`);
      for (let i = 0; i < txs; i++) {
        const deposit2 = await chainASignatureAnchor.deposit(chainB.chainId);
        console.log('Deposit on chain A (signature): ', deposit2.deposit);
      }
      return;
    }

    if (cmd.match(/^spam chain b (\d+)$/)) {
      const txs = parseInt(cmd.match(/^spam chain b (\d+)$/)?.[1] ?? '1');
      console.log(`Spamming Chain B with ${txs}, please wait...`);
      for (let i = 0; i < txs; i++) {
        const deposit2 = await chainBSignatureAnchor.deposit(chainA.chainId);
        console.log('Deposit on chain B (signature): ', deposit2.deposit);
      }
      return;
    }

    if (cmd.match(/^transfer ownership to ([0-9a-f]+)$/i)) {
      let addr = cmd.match(/^transfer ownership to ([0-9a-f]+)$/i)?.[1];
      addr = publicKeyToAddress(addr);
      console.log('Setting the Signature Bridge Governer to', addr);
      if (!addr) {
        console.log('Invalid Public Key');
        return;
      }
      let contract: SignatureBridgeContract;
      contract = chainASignatureBridge.contract;
      let tx = await contract.transferOwnership(addr, 1);
      let result = await tx.wait();
      console.log(result);
      contract = chainBSignatureBridge.contract;
      tx = await contract.transferOwnership(addr, 1);
      result = await tx.wait();
      console.log(result);
      console.log('New Signature Bridge Owner (on both chains) is now set to', addr);
      return;
    }

    if (cmd.startsWith('root on chain a')) {
      console.log('Root on chain A (signature), please wait...');
      const root2 = await chainASignatureAnchor.contract.getLastRoot();
      const latestNeighborRoots2 =
        await chainASignatureAnchor.contract.getLatestNeighborRoots();
      console.log('Root on chain A (signature): ', root2);
      console.log(
        'Latest neighbor roots on chain A (signature): ',
        latestNeighborRoots2
      );
      return;
    }

    if (cmd.startsWith('root on chain b')) {
      console.log('Root on chain B (signature), please wait...');
      const root2 = await chainBSignatureAnchor.contract.getLastRoot();
      const latestNeighborRoots2 =
        await chainBSignatureAnchor.contract.getLatestNeighborRoots();
      console.log('Root on chain B (signature): ', root2);
      console.log(
        'Latest neighbor roots on chain B (signature): ',
        latestNeighborRoots2
      );
      return;
    }

    console.log('Unknown command: ', cmd);
    printAvailableCommands();
  });
}

function printAvailableCommands() {
  console.log('Available commands:');
  console.log('  deposit on chain a');
  console.log('  deposit on chain b');
  console.log('  withdraw on chain a');
  console.log('  withdraw on chain b');
  console.log('  root on chain a');
  console.log('  root on chain b');
  console.log('  spam chain a <txs>');
  console.log('  spam chain b <txs>');
  console.log('  transfer ownership to <pubkey>');
  console.log('  exit');
}

main().catch(console.error);
