import axios from 'axios';

describe('GET /roots/latest', (): void => {
    const archiverUrl = (global as any).ARCHIVER_URL;

    test('returns a response', async (): Promise<void> => {
        const statusResponse = await axios.get(`${archiverUrl}/status`);
        expect(statusResponse.status).toBe(200);

        const rootsResponse = await axios.get(`${archiverUrl}/roots/latest`);
        expect(rootsResponse.status).toBe(200);

        expect(rootsResponse.data).toHaveProperty('latest_block');
        expect(rootsResponse.data.latest_block).toBeGreaterThanOrEqual(statusResponse.data.latest_archived_block);
    });
});
