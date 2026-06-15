import axios from 'axios';

describe('GET /status', (): void => {
    const archiverUrl = (global as any).ARCHIVER_URL;

    test('returns a response', async (): Promise<void> => {
        const response = await axios.get(`${archiverUrl}/status`);
        expect(response.status).toBe(200);

        expect(response.data).toHaveProperty('latest_archived_block');
        expect(response.data.latest_archived_block).toBeGreaterThan(0);

        expect(response.data).toHaveProperty('total_blocks');
        expect(response.data.total_blocks).toBeGreaterThan(0);
    });
});
