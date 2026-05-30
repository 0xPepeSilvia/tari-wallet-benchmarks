// Thin async wrapper around the generated tari.rpc.Wallet gRPC client.
//
// prost/tonic generated code lives in the `wallet_pb` module.
// This wrapper handles endpoint construction and adds convenience methods.

use anyhow::{Context, Result};
use tonic::transport::Channel;
use tonic::Request;

/// Re-export generated protobuf types so callers don't need to import build paths.
pub mod wallet_pb {
    tonic::include_proto!("tari.rpc");
}

use wallet_pb::wallet_client::WalletClient;
use wallet_pb::{
    CoinSplitRequest, Empty, GetBalanceRequest, GetCompletedTransactionsRequest,
    GetTransactionInfoRequest, PaymentRecipient, RescanWalletRequest,
    TransferRequest,
};

pub struct WalletGrpcClient {
    inner: WalletClient<Channel>,
}

impl WalletGrpcClient {
    /// Connect to a wallet gRPC endpoint, e.g. "http://127.0.0.1:18143".
    pub async fn connect(addr: &str) -> Result<Self> {
        let channel = Channel::from_shared(addr.to_string())
            .with_context(|| format!("Invalid gRPC address: {}", addr))?
            .connect()
            .await
            .with_context(|| format!("Failed to connect gRPC channel to {}", addr))?;
        Ok(Self {
            inner: WalletClient::new(channel),
        })
    }

    pub async fn get_address(&self) -> Result<wallet_pb::GetAddressResponse> {
        let mut c = self.inner.clone();
        let resp = c.get_address(Request::new(Empty {}))
            .await
            .context("get_address RPC failed")?;
        Ok(resp.into_inner())
    }

    pub async fn get_balance(&self) -> Result<wallet_pb::GetBalanceResponse> {
        let mut c = self.inner.clone();
        let resp = c.get_balance(Request::new(GetBalanceRequest {}))
            .await
            .context("get_balance RPC failed")?;
        Ok(resp.into_inner())
    }

    pub async fn get_state(&self) -> Result<wallet_pb::WalletStateResponse> {
        let mut c = self.inner.clone();
        let resp = c.get_state(Request::new(Empty {}))
            .await
            .context("get_state RPC failed")?;
        Ok(resp.into_inner())
    }

    pub async fn transfer(
        &self,
        recipients: Vec<PaymentRecipient>,
    ) -> Result<wallet_pb::TransferResponse> {
        let mut c = self.inner.clone();
        let resp = c.transfer(Request::new(TransferRequest { recipients }))
            .await
            .context("transfer RPC failed")?;
        Ok(resp.into_inner())
    }

    pub async fn coin_split(
        &self,
        amount_per_split: u64,
        split_count: u32,
        fee_per_gram: u64,
    ) -> Result<wallet_pb::CoinSplitResponse> {
        let mut c = self.inner.clone();
        let resp = c.coin_split(Request::new(CoinSplitRequest {
            amount_per_split,
            split_count: split_count as u64,
            fee_per_gram,
            message: "benchmark-split".to_string(),
            lock_height: 0,
        }))
        .await
        .context("coin_split RPC failed")?;
        Ok(resp.into_inner())
    }

    pub async fn rescan_wallet(&self, from_height: i64) -> Result<wallet_pb::RescanWalletResponse> {
        let mut c = self.inner.clone();
        let resp = c.rescan_wallet(Request::new(RescanWalletRequest { from_height }))
            .await
            .context("rescan_wallet RPC failed")?;
        Ok(resp.into_inner())
    }

    pub async fn get_transaction_info(
        &self,
        transaction_ids: Vec<u64>,
    ) -> Result<wallet_pb::GetTransactionInfoResponse> {
        let mut c = self.inner.clone();
        let resp = c.get_transaction_info(Request::new(GetTransactionInfoRequest {
            transaction_ids,
        }))
        .await
        .context("get_transaction_info RPC failed")?;
        Ok(resp.into_inner())
    }

    #[allow(dead_code)]
    pub async fn get_completed_transactions(&self) -> Result<Vec<wallet_pb::TransactionInfo>> {
        let mut c = self.inner.clone();
        let mut stream = c
            .get_completed_transactions(Request::new(GetCompletedTransactionsRequest {}))
            .await
            .context("get_completed_transactions RPC failed")?
            .into_inner();

        let mut txs = Vec::new();
        while let Some(msg) = stream.message().await? {
            if let Some(tx) = msg.transaction {
                txs.push(tx);
            }
        }
        Ok(txs)
    }
}
