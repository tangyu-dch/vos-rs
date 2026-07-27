use crate::edge_state::EdgeState;
use crate::sip::{transaction, InviteAckKey, RequestTransactionKey};

impl EdgeState {
    pub(crate) fn register_server_transaction(
        &self,
        key: RequestTransactionKey,
        tx: tokio::sync::mpsc::Sender<transaction::ServerTransactionEvent>,
    ) {
        if let Some(ack_key) = key.invite_ack_key() {
            self.invite_ack_transactions
                .insert(ack_key, (key.clone(), tx.clone()));
        }
        self.server_transactions.insert(key, tx);
    }

    pub(crate) fn take_invite_ack_transaction(
        &self,
        key: &InviteAckKey,
    ) -> Option<tokio::sync::mpsc::Sender<transaction::ServerTransactionEvent>> {
        let (_, (transaction_key, tx)) = self.invite_ack_transactions.remove(key)?;
        self.server_transactions.remove(&transaction_key);
        (!tx.is_closed()).then_some(tx)
    }

    pub(crate) fn get_server_transaction(
        &self,
        key: &RequestTransactionKey,
    ) -> Option<tokio::sync::mpsc::Sender<transaction::ServerTransactionEvent>> {
        if let Some(tx) = self.server_transactions.get(key) {
            if tx.is_closed() {
                drop(tx);
                self.server_transactions.remove(key);
                if let Some(ack_key) = key.invite_ack_key() {
                    self.invite_ack_transactions.remove(&ack_key);
                }
                None
            } else {
                Some(tx.clone())
            }
        } else {
            None
        }
    }
}
