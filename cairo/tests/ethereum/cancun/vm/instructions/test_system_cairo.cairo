from starkware.cairo.common.cairo_builtins import BitwiseBuiltin, KeccakBuiltin, PoseidonBuiltin
from starkware.cairo.common.registers import get_label_location

from ethereum.cancun.vm.instructions.system import generic_create
from ethereum.cancun.vm.interpreter import process_create_message, process_message
from ethereum_types.numeric import U256, Uint
from ethereum.cancun.fork_types import Address
from ethereum.cancun.vm.exceptions import EthereumException
from ethereum.cancun.vm import Evm

func test_generic_create{
    range_check_ptr,
    bitwise_ptr: BitwiseBuiltin*,
    keccak_ptr: KeccakBuiltin*,
    poseidon_ptr: PoseidonBuiltin*,
    evm: Evm,
}(
    endowment: U256,
    contract_address: Address,
    memory_start_position: U256,
    memory_size: U256,
    init_code_gas: Uint,
) -> EthereumException* {
    let (process_create_message_label) = get_label_location(process_create_message);
    let (process_message_label) = get_label_location(process_message);
    let res = generic_create{process_create_message_label=process_create_message_label, evm=evm}(
        endowment, contract_address, memory_start_position, memory_size, init_code_gas
    );
    return res;
}
