// Human-readable ABIs for the bits the UI touches.

export const ERC20_ABI = [
  "function balanceOf(address) view returns (uint256)",
  "function allowance(address owner, address spender) view returns (uint256)",
  "function approve(address spender, uint256 amount) returns (bool)",
  "function decimals() view returns (uint8)",
  "function symbol() view returns (string)",
];

export const DEST_BRIDGE_ABI = [
  "function lock(uint256 amount, address ccRecipient)",
  "event Locked(address indexed ccRecipient, uint256 amount, uint256 nonce)",
  "event Released(address indexed recipient, uint256 amount, bytes32 indexed messageId)",
];

export const CC_BRIDGE_ABI = [
  "function withdraw(uint256 amount, address destRecipient) returns (bytes32 messageId)",
  "function claim(uint64 height, bytes encodedTransaction, (bytes32 root, (bytes32 hash, bool isLeft)[] siblings) merkleProof, (bytes32 lowerEndpointDigest, bytes32[] roots) continuityProof)",
  "event Withdrawn(address indexed destRecipient, uint256 amount, bytes32 messageId)",
  "event Claimed(address indexed ccRecipient, uint256 amount, uint256 nonce)",
  "function claimed(bytes32) view returns (bool)",
];
