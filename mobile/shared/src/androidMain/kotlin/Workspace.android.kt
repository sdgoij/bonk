package com.github.sdgoij.bonk

actual fun listCustomers(): List<Customer> = withUserError { MobileArchiveHost.listCustomers() }

actual fun saveCustomer(customer: Customer): Customer = withUserError { MobileArchiveHost.saveCustomer(customer) }

actual fun listDrafts(): List<Draft> = withUserError { MobileArchiveHost.listDrafts() }

actual fun loadDraft(id: String): Draft = withUserError { MobileArchiveHost.loadDraft(id) }

actual fun saveDraft(draft: Draft): Draft = withUserError { MobileArchiveHost.saveDraft(draft) }

actual fun validateDraft(draft: Draft): List<Finding> = withUserError { MobileArchiveHost.validateDraft(draft) }

actual fun draftTotals(draft: Draft): DraftTotals = withUserError { MobileArchiveHost.draftTotals(draft) }

actual fun newDraft(customerId: String): Draft = withUserError { MobileArchiveHost.newDraft(customerId) }

actual fun issueDraft(draft: Draft): IssueResult = withUserError { MobileArchiveHost.issueDraft(draft) }

actual fun listDocuments(query: String, status: String?, direction: String): List<DocumentHit> =
    withUserError { MobileArchiveHost.listDocuments(query, status, direction) }

actual fun matchSupplier(documentId: String, customerId: String) = withUserError {
    MobileArchiveHost.matchSupplier(documentId, customerId)
}

actual fun advanceDocument(documentId: String, target: String): String = withUserError {
    MobileArchiveHost.advanceDocument(documentId, target)
}

actual fun loadDocument(id: String): DocumentDetail = withUserError { MobileArchiveHost.loadDocument(id) }

actual fun loadSeller(): Seller = withUserError { MobileArchiveHost.loadSeller() }

actual fun saveSeller(seller: Seller): Seller = withUserError { MobileArchiveHost.saveSeller(seller) }

actual fun documentArtefact(id: String, kind: String): ByteArray = withUserError {
    MobileArchiveHost.documentArtefact(id, kind)
}

actual fun newId(): String = java.util.UUID.randomUUID().toString()

actual fun customersCsv(): String = withUserError { MobileArchiveHost.customersCsv() }

actual fun importCustomersCsv(text: String): CustomerImportReport =
    withUserError { MobileArchiveHost.importCustomersCsv(text) }

actual fun importDocument(name: String, bytes: ByteArray): ImportReport = withUserError {
    MobileArchiveHost.importDocument(name, bytes)
}

actual fun writeBackup(): BackupExport = withUserError { MobileArchiveHost.writeBackup() }

actual fun chainHead(): ChainHead? = withUserError { MobileArchiveHost.chainHead() }

actual fun restoreArchive(passphrase: String): ArchiveSummary = withUserError {
    MobileArchiveHost.restore(passphrase)
}

actual fun recoveryKey(): ByteArray = withUserError { MobileArchiveHost.recoveryKey() }

actual fun recoveryNote(): String = withUserError { MobileArchiveHost.recoveryNote() }

actual fun freeSpaceBytes(): Long = MobileArchiveHost.freeSpaceBytes()

actual fun databaseBytes(): Long = MobileArchiveHost.databaseBytes()

actual fun suppressFinding(draftId: String, code: String, field: String, reason: String) =
    withUserError { MobileArchiveHost.suppressFinding(draftId, code, field, reason) }
