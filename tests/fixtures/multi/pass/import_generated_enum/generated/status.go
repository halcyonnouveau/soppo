package generated

/*soppo:enum
Status {
    Pending
    Active
    Complete
}
*/
type Status interface {
	isStatus()
}

type Status_Pending struct{}

func (Status_Pending) isStatus() {}

type Status_Active struct{}

func (Status_Active) isStatus() {}

type Status_Complete struct{}

func (Status_Complete) isStatus() {}

var StatusPending Status = Status_Pending{}
var StatusActive Status = Status_Active{}
var StatusComplete Status = Status_Complete{}
