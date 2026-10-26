import { EthHead, readSubscribedBlock } from '../blockchain-tests/eth-new-heads.helpers';

describe('subscribed block lookup', () => {
    // eslint-disable-next-line id-denylist
    const head: EthHead = { hash: '0xaaa', number: '0x10', timestamp: '0x123' };

    test('returns a readable subscribed block', async () => {
        const read = jest.fn().mockResolvedValue(head);
        await expect(readSubscribedBlock(read, head)).resolves.toEqual(head);
        expect(read).toHaveBeenCalledWith('eth_getBlockByHash', [head.hash, false]);
        expect(read).toHaveBeenCalledTimes(1);
    });

    test('recognizes a reorg only when the canonical hash confirms the replacement', async () => {
        const read = jest
            .fn()
            .mockResolvedValueOnce(null)
            .mockResolvedValueOnce({ ...head, hash: '0xbbb' });
        await expect(readSubscribedBlock(read, head)).resolves.toBeNull();
        expect(read).toHaveBeenLastCalledWith('eth_getBlockByNumber', [head.number, false]);
    });

    test('waits for a temporarily unreadable head that is still canonical', async () => {
        const read = jest.fn().mockResolvedValueOnce(null).mockResolvedValueOnce(head).mockResolvedValueOnce(head);
        await expect(readSubscribedBlock(read, head)).resolves.toEqual(head);
        expect(read).toHaveBeenCalledTimes(3);
    });

    test.each(['missing', 'still canonical'])('fails if a %s block never becomes readable', async (state) => {
        const read = jest
            .fn()
            .mockImplementation((method) =>
                Promise.resolve(method === 'eth_getBlockByNumber' && state === 'still canonical' ? head : null),
            );
        await expect(readSubscribedBlock(read, head, 20)).rejects.toThrow('was not readable or replaced');
    });

    test('propagates RPC errors', async () => {
        const read = jest.fn().mockRejectedValue(new Error('RPC failed'));
        await expect(readSubscribedBlock(read, head)).rejects.toThrow('RPC failed');
    });
});
