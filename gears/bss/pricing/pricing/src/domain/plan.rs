//! Plans: revisions bound to one book and their items (D-394, D-407).
string_enum!(RevisionState {Draft=>"draft", Pending=>"pending", Published=>"published", Superseded=>"superseded"});
string_enum!(Treatment {Paid=>"paid", Optional=>"optional", Included=>"included"});
// A copied item starts `unreserved` and attaches after its write (D-413).
string_enum!(ReferenceState {Unreserved=>"unreserved", ConfirmationPending=>"confirmation_pending", Confirmed=>"confirmed", Lost=>"lost"});
