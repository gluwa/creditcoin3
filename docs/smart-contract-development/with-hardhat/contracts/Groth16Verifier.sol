pragma solidity >=0.7.0 <0.9.0;

contract Groth16Verifier {
    // Scalar field size
    uint256 constant r    = 21888242871839275222246405745257275088548364400416034343698204186575808495617;
    // Base field size
    uint256 constant q   = 21888242871839275222246405745257275088696311157297823662689037894645226208583;

    // Verification Key data
    uint256 constant ALPHAX  = 1638319440715124628720745116249450843109300928307144759936909695427245903578;
    uint256 constant ALPHAY  = 7415255223858717597613570926677866522168152274810220441133216999009624869407;
    uint256 constant BETAX1  = 4873746966627585691527604013216848483227014543726482017414106206028328781977;
    uint256 constant BETAX2  = 21668486952265793686819803558643628680123056840870168324220349307721544015646;
    uint256 constant BETAY1  = 6557179478584731071600368073083165243106461964700099823858727909387173759542;
    uint256 constant BETAY2  = 3226691731509002396835232984901095251637681645255095478809011230854179645610;
    uint256 constant GAMMAX1 = 11559732032986387107991004021392285783925812861821192530917403151452391805634;
    uint256 constant GAMMAX2 = 10857046999023057135944570762232829481370756359578518086990519993285655852781;
    uint256 constant GAMMAY1 = 4082367875863433681332203403145435568316851327593401208105741076214120093531;
    uint256 constant GAMMAY2 = 8495653923123431417604973247489272438418190587263600148770280649306958101930;
    uint256 constant DELTAX1 = 2576148027623414742168595015573226930685076310898855947549849220355611125031;
    uint256 constant DELTAX2 = 6877069538226375294349630371383230331924464208358689289465335072520874586699;
    uint256 constant DELTAY1 = 3057775199914715273073024342302653043575212131354606978160572901473896209522;
    uint256 constant DELTAY2 = 4594307129725976547371426980183134424245109191208414770598671084680091698752;


    uint256 constant IC0X = 21508425505371447063112995735001322817356598459019035840908066662360762159974;
    uint256 constant IC0Y = 1549941607314044996788923157735781963851379769183493662247371505416016026565;

    uint256 constant IC1X = 967662568466740105277352447499533796694939262829553672439748370975722403002;
    uint256 constant IC1Y = 9114807833282041115614686766566246862906761734640324514034673242447766867829;


    // Memory data
    uint16 constant P_VK = 0;
    uint16 constant P_PAIRING = 128;

    uint16 constant P_LAST_MEM = 896;

    function verifyProof(uint[2] calldata _pA, uint[2][2] calldata _pB, uint[2] calldata _pC, uint[1] calldata _pubSignals) public view returns (bool) {
        assembly {
            function checkField(v) {
                if iszero(lt(v, r)) {
                    mstore(0, 0)
                    return(0, 0x20)
                }
            }

            // G1 function to multiply a G1 value(x,y) to value in an address
            function g1_mulAccC(pR, x, y, s) {
                let success
                let mIn := mload(0x40)
                mstore(mIn, x)
                mstore(add(mIn, 32), y)
                mstore(add(mIn, 64), s)

                success := staticcall(sub(gas(), 2000), 7, mIn, 96, mIn, 64)

                if iszero(success) {
                    mstore(0, 0)
                    return(0, 0x20)
                }

                mstore(add(mIn, 64), mload(pR))
                mstore(add(mIn, 96), mload(add(pR, 32)))

                success := staticcall(sub(gas(), 2000), 6, mIn, 128, pR, 64)

                if iszero(success) {
                    mstore(0, 0)
                    return(0, 0x20)
                }
            }

            function checkPairing(pA, pB, pC, pubSignals, pMem) -> isOk {
                let _pPairing := add(pMem, P_PAIRING)
                let _pVk := add(pMem, P_VK)

                mstore(_pVk, IC0X)
                mstore(add(_pVk, 32), IC0Y)

                // Compute the linear combination vk_x

                g1_mulAccC(_pVk, IC1X, IC1Y, calldataload(add(pubSignals, 0)))


                // -A
                mstore(_pPairing, calldataload(pA))
                mstore(add(_pPairing, 32), mod(sub(q, calldataload(add(pA, 32))), q))

                // B
                mstore(add(_pPairing, 64), calldataload(pB))
                mstore(add(_pPairing, 96), calldataload(add(pB, 32)))
                mstore(add(_pPairing, 128), calldataload(add(pB, 64)))
                mstore(add(_pPairing, 160), calldataload(add(pB, 96)))

                // alpha1
                mstore(add(_pPairing, 192), ALPHAX)
                mstore(add(_pPairing, 224), ALPHAY)

                // beta2
                mstore(add(_pPairing, 256), BETAX1)
                mstore(add(_pPairing, 288), BETAX2)
                mstore(add(_pPairing, 320), BETAY1)
                mstore(add(_pPairing, 352), BETAY2)

                // vk_x
                mstore(add(_pPairing, 384), mload(add(pMem, P_VK)))
                mstore(add(_pPairing, 416), mload(add(pMem, add(P_VK, 32))))


                // gamma2
                mstore(add(_pPairing, 448), GAMMAX1)
                mstore(add(_pPairing, 480), GAMMAX2)
                mstore(add(_pPairing, 512), GAMMAY1)
                mstore(add(_pPairing, 544), GAMMAY2)

                // C
                mstore(add(_pPairing, 576), calldataload(pC))
                mstore(add(_pPairing, 608), calldataload(add(pC, 32)))

                // delta2
                mstore(add(_pPairing, 640), DELTAX1)
                mstore(add(_pPairing, 672), DELTAX2)
                mstore(add(_pPairing, 704), DELTAY1)
                mstore(add(_pPairing, 736), DELTAY2)


                let success := staticcall(sub(gas(), 2000), 8, _pPairing, 768, _pPairing, 0x20)

                isOk := and(success, mload(_pPairing))
            }

            let pMem := mload(0x40)
            mstore(0x40, add(pMem, P_LAST_MEM))

            // Validate that all evaluations ∈ F

            checkField(calldataload(add(_pubSignals, 0)))


            // Validate all evaluations
            let isValid := checkPairing(_pA, _pB, _pC, _pubSignals, pMem)

            mstore(0, isValid)
            return(0, 0x20)
        }
    }
}
