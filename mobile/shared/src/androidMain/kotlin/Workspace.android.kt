package com.github.sdgoij.bonk

actual fun listCustomers(): List<Customer> = withUserError { MobileArchiveHost.listCustomers() }

actual fun saveCustomer(customer: Customer): Customer = withUserError { MobileArchiveHost.saveCustomer(customer) }

actual fun listDrafts(): List<Draft> = withUserError { MobileArchiveHost.listDrafts() }

actual fun loadDraft(id: String): Draft = withUserError { MobileArchiveHost.loadDraft(id) }

actual fun saveDraft(draft: Draft): Draft = withUserError { MobileArchiveHost.saveDraft(draft) }

actual fun validateDraft(draft: Draft): List<Finding> = withUserError { MobileArchiveHost.validateDraft(draft) }

actual fun draftTotals(draft: Draft): DraftTotals = withUserError { MobileArchiveHost.draftTotals(draft) }

actual fun newDraft(customerId: String): Draft = withUserError { MobileArchiveHost.newDraft(customerId) }

actual fun newId(): String = java.util.UUID.randomUUID().toString()
