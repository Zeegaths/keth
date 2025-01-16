from typing import Optional

import pytest
from ethereum_types.bytes import Bytes32
from ethereum_types.numeric import U256
from hypothesis import given
from hypothesis import strategies as st
from hypothesis.strategies import composite

from ethereum.cancun.fork_types import Account, Address
from ethereum.cancun.state import (
    account_exists,
    account_exists_and_is_empty,
    account_has_code_or_nonce,
    destroy_account,
    destroy_storage,
    get_account,
    get_account_optional,
    get_storage,
    get_storage_original,
    get_transient_storage,
    is_account_alive,
    is_account_empty,
    mark_account_created,
    set_account,
    set_storage,
    set_transient_storage,
)
from tests.utils.args_gen import TransientStorage
from tests.utils.strategies import address, bytes32, state, transient_storage

pytestmark = pytest.mark.python_vm


@composite
def state_and_address_and_optional_key(
    draw, state_strategy=state, address_strategy=address, key_strategy=None
):
    state = draw(state_strategy)

    # For address selection, shuffle from one of the following strategies
    address_options = []
    if state._main_trie._data:
        address_options.append(st.sampled_from(list(state._main_trie._data.keys())))
    if state.created_accounts:
        address_options.append(st.sampled_from(list(state.created_accounts)))
    address_options.append(address_strategy)

    address = draw(st.one_of(*address_options))

    # For key selection, use key_strategy if no storage keys for this address
    if key_strategy is None:
        return state, address

    storage = state._storage_tries.get(address)
    key_options = (
        [st.sampled_from(list(storage._data.keys())), key_strategy]
        if storage is not None and storage._data != {}
        else [key_strategy]
    )
    key = draw(st.one_of(*key_options))

    return state, address, key


class TestStateAccounts:
    @given(data=state_and_address_and_optional_key())
    def test_get_account(self, cairo_run, data):
        state, address = data
        state_cairo, result_cairo = cairo_run("get_account", state, address)
        assert result_cairo == get_account(state, address)
        assert state_cairo == state

    @given(data=state_and_address_and_optional_key())
    def test_get_account_optional(self, cairo_run, data):
        state, address = data
        state_cairo, result_cairo = cairo_run("get_account_optional", state, address)
        assert result_cairo == get_account_optional(state, address)
        assert state_cairo == state

    @given(data=state_and_address_and_optional_key(), account=...)
    def test_set_account(self, cairo_run, data, account: Optional[Account]):
        state, address = data
        state_cairo = cairo_run("set_account", state, address, account)
        set_account(state, address, account)
        assert state_cairo == state

    @given(data=state_and_address_and_optional_key())
    def test_destroy_account(self, cairo_run, data):
        state, address = data
        state_cairo = cairo_run("destroy_account", state, address)
        destroy_account(state, address)
        assert state_cairo == state

    @given(data=state_and_address_and_optional_key())
    def test_account_has_code_or_nonce(self, cairo_run, data):
        state, address = data
        state_cairo, result_cairo = cairo_run(
            "account_has_code_or_nonce", state, address
        )
        assert result_cairo == account_has_code_or_nonce(state, address)
        assert state_cairo == state

    @given(data=state_and_address_and_optional_key())
    def test_account_exists(self, cairo_run, data):
        state, address = data
        state_cairo, result_cairo = cairo_run("account_exists", state, address)
        assert result_cairo == account_exists(state, address)
        assert state_cairo == state

    @given(data=state_and_address_and_optional_key())
    def test_is_account_empty(self, cairo_run, data):
        state, address = data
        state_cairo, result_cairo = cairo_run("is_account_empty", state, address)
        assert result_cairo == is_account_empty(state, address)
        assert state_cairo == state

    @given(data=state_and_address_and_optional_key())
    def test_mark_account_created(self, cairo_run, data):
        state, address = data
        state_cairo = cairo_run("mark_account_created", state, address)
        mark_account_created(state, address)
        assert state_cairo == state

    @given(data=state_and_address_and_optional_key())
    def test_account_exists_and_is_empty(self, cairo_run, data):
        state, address = data
        state_cairo, result_cairo = cairo_run(
            "account_exists_and_is_empty", state, address
        )
        assert result_cairo == account_exists_and_is_empty(state, address)
        assert state_cairo == state

    @given(data=state_and_address_and_optional_key())
    def test_is_account_alive(self, cairo_run, data):
        state, address = data
        state_cairo, result_cairo = cairo_run("is_account_alive", state, address)
        assert result_cairo == is_account_alive(state, address)
        assert state_cairo == state


class TestStateStorage:
    @given(state_and_address_and_optional_key(key_strategy=bytes32))
    def test_get_storage_original(self, cairo_run, data):
        state, address, key = data
        state_cairo, result_cairo = cairo_run(
            "get_storage_original", state, address, key
        )
        assert result_cairo == get_storage_original(state, address, key)
        assert state_cairo == state

    @given(data=state_and_address_and_optional_key(key_strategy=bytes32))
    def test_get_storage(
        self,
        cairo_run,
        data,
    ):
        state, address, key = data
        state_cairo, result_cairo = cairo_run("get_storage", state, address, key)
        assert result_cairo == get_storage(state, address, key)
        assert state_cairo == state

    @given(data=state_and_address_and_optional_key(key_strategy=bytes32), value=...)
    def test_set_storage(self, cairo_run, data, value: U256):
        state, address, key = data
        try:
            state_cairo = cairo_run("set_storage", state, address, key, value)
        except Exception as e:
            with pytest.raises(type(e)):
                set_storage(state, address, key, value)
            return

        set_storage(state, address, key, value)
        assert state_cairo == state

    @given(data=state_and_address_and_optional_key())
    def test_destroy_storage(self, cairo_run, data):
        state, address = data
        state_cairo = cairo_run("destroy_storage", state, address)
        destroy_storage(state, address)
        assert state_cairo == state


class TestTransientStorage:
    @given(
        transient_storage=transient_storage,
        address=...,
        key=...,
    )
    def test_get_transient_storage(
        self,
        cairo_run,
        transient_storage: TransientStorage,
        address: Address,
        key: Bytes32,
    ):
        transient_storage_cairo, result_cairo = cairo_run(
            "get_transient_storage",
            transient_storage,
            address,
            key,
        )
        assert result_cairo == get_transient_storage(transient_storage, address, key)
        assert transient_storage_cairo == transient_storage

    @given(
        transient_storage=transient_storage,
        address=...,
        key=...,
        value=...,
    )
    def test_set_transient_storage(
        self,
        cairo_run,
        transient_storage: TransientStorage,
        address: Address,
        key: Bytes32,
        value: U256,
    ):
        transient_storage_cairo = cairo_run(
            "set_transient_storage",
            transient_storage,
            address,
            key,
            value,
        )
        set_transient_storage(transient_storage, address, key, value)
        assert transient_storage_cairo == transient_storage
