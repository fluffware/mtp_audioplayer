function build_ws_uri(path) {
    let loc = window.location;
    var ws_uri = (loc.protocol === "https:") ? "wss:" : "ws:"
    ws_uri += "//" + loc.host;
    return ws_uri + "/" + path;
}

function add_const_field(row, class_name, value)
{
    let col = document.createElement("td");
    col.classList.add(class_name);
    col.innerText = value;
    row.appendChild(col);
}

class AlarmList
{
    constructor(table_elem) {
	this.table_elem = table_elem;
	let cookie = Math.round(Math.random()*1e9).toString() + "-" + Date.now();
	let list = this;
	let socket = new WebSocket(build_ws_uri("open_pipe"));
	socket.onerror = function(event) {
	    console.log("Websocket error:", event);
	}
	socket.onopen = function(msg) {
	    socket.send(JSON.stringify({Message: "SubscribeAlarm",
					Params: {
					},
					ClientCookie: cookie
				       }));
	}
	socket.onmessage = function(ws_msg) {
	    let msg = JSON.parse(ws_msg.data);
	    console.log("Received message: ",msg);
	    switch(msg.Message) {
	    case "NotifySubscribeAlarm":
		list.update_list(msg.Params.Alarms);
		break;
	    }
	}
	this.socket = socket;
	this.cookie = cookie;
	    

    }
    
    build_list(alarms) {
	let table = this.table_elem.getElementsByTagName("tbody")[0];
	table.innerHTML = '';
	this.update_list(alarms);
    }
    
    update_list(alarms)
    {
	let table = this.table_elem.getElementsByTagName("tbody")[0];
	for (let a of alarms) {
	    let row = table.querySelector("tr[alarm_id='"+a.ID+"']");
	    if (row) {
			if (a.State != 128) {
				let alarm_state_elem =  row.getElementsByClassName("alarm_state")[0];
				alarm_state_elem.value = a.State;
			}
	    } else {
		
		let row = document.createElement("tr");
		row.setAttribute("alarm_id", a.ID);
		table.appendChild(row);

		add_const_field(row, "alarm_id", a.ID);
		add_const_field(row, "alarm_instance_id", a.InstanceID);
		add_const_field(row, "alarm_name", a.Name);
		add_const_field(row, "alarm_class_name", a.AlarmClassName);
		add_const_field(row, "alarm_class_symbol", a.AlarmClassSymbol);
		add_const_field(row, "alarm_state_text", a.StateText);
		add_const_field(row, "alarm_state_machine", a.StateMachine);
		add_const_field(row, "alarm_event_text", a.EventText);
		add_const_field(row, "alarm_priority", a.Priority);
		
		let state_col = document.createElement("td");
		row.appendChild(state_col);
		let state_input = document.createElement("select");
		state_input.classList.add("alarm_state");
		state_col.appendChild(state_input);
		for (let opt of [[0,"Normal"],
				 [1,"In"],
				 [2,"In/Out"],
				 [5,"In/Ack"],
				 [6,"In/Ack/Out"],
				 [7,"In/Out/Ack"],
				 [8,"Removed"]
				]) {
		    let opt_elem = document.createElement("option");
		    opt_elem.setAttribute("value", opt[0]);
		    opt_elem.innerText =opt[1];
		    state_input.appendChild(opt_elem);
		}
		state_input.value = a.State;
		let socket = this.socket;
		let cookie = this.cookie;
		state_input.addEventListener("change", function() {
		    let iso_date = (new Date()).toISOString();
		    let date = iso_date.slice(0,10) + " " + iso_date.slice(11, 19);
		    let msg = {Message: "NotifySubscribeAlarm",
			       Params: {
			       Alarms: [
				   { ID: a.ID.toString(),
				     InstanceID: a.InstanceID.toString(),
				     Name: a.Name,
				     State: state_input.value.toString(),
				     StateText: a.StateText,
				     StateMachine: a.StateMachine,
				     Name: a.Name,
				     AlarmClassName: a.AlarmClassName,
				     AlarmClassSymbol: a.AlarmClassSymbol,
				     EventText: a.EventText,
				     Priority: a.Priority.toString(),
				     ModificationTime: date
				   }
			       ]},
			       ClientCookie: cookie
			      };
		    socket.send(JSON.stringify(msg));
		    
		});
	    }
	}
    }

    add_alarm()
    {
	let iso_date = (new Date()).toISOString();
	let date = iso_date.slice(0,10) + " " + iso_date.slice(11, 19);
	let msg = {
	    Message: "NotifySubscribeAlarm",
	    Params: {
		Alarms: [{
		    ID: document.getElementById("new_alarm_id").value,
		    InstanceID: document.getElementById("new_instance_id").value,
		    Name: document.getElementById("new_alarm_name").value,
		    AlarmClassName: document.getElementById("new_alarm_class").value,
		    AlarmClassSymbol: document.getElementById("new_alarm_symbol").value,
		    StateText: document.getElementById("new_state_text").value,
		    StateMachine: document.getElementById("new_state_machine").value,
		    EventText: document.getElementById("new_event_text").value,
		    StateText: document.getElementById("new_state_text").value,
		    Priority: document.getElementById("new_priority").value,
		    State: document.getElementById("new_state").value,
		    ModificationTime: date
		}],
	    },
	    ClientCookie: this.cookie
	};
	this.socket.send(JSON.stringify(msg)); 
	this.update_list(msg.Params.Alarms);
    }
}
