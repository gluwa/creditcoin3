import { Command, OptionValues } from 'commander'
import { newApi } from '../api'

export function makeStatusCommand() {
    const cmd = new Command('status')
    cmd.description('Get chain status')
    cmd.action(statusAction)
    return cmd
}

async function statusAction(options: OptionValues) {
    const { api } = await newApi(options.url)

    const bestBlock = await api.rpc.chain.getBlock()
    const blockNumber = bestBlock.block.header.number.toNumber()
    console.log(`Best block number: ${blockNumber}`)

    process.exit(0)
}
