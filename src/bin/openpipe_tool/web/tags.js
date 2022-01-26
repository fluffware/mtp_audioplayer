
function build_ws_uri(path) {
	let loc = window.location;
	var ws_uri = (loc.protocol === "https:") ? "wss:" : "ws:"
	ws_uri += "//" + loc.host;
	return ws_uri + "/" + path;
}

class TagList {
	constructor(table_elem) {
		this.table_elem = table_elem;

		let list = this;
		let socket = new WebSocket(build_ws_uri("open_pipe"));
		socket.onerror = function (event) {
			console.log("Websocket error:", event);
		}

		socket.onmessage = function (ws_msg) {
			let msg = JSON.parse(ws_msg.data);
			console.log("Received message: ", msg);
			switch (msg.Message) {
				case "NotifySubscribeTag":
					list.update_list(msg.Params.Tags);
					break;
			}
		}
		this.socket = socket;
	}

	get_cookie() {
		return Math.round(Math.random() * 1e9).toString() + "-" + Date.now();
	}

	build_list(tags) {
		let table = this.table_elem.getElementsByTagName("tbody")[0];
		table.innerHTML = '';
		this.update_list(tags);
	}

	update_list(tags) {
		for (let t of tags) {
			let table = this.table_elem.getElementsByTagName("tbody")[0];
			let row = table.querySelector("tr[tag_name='" + t.Name + "']");
			if (row) {
				let name_elem = row.getElementsByClassName("tag_name")[0];
				name_elem.innerText = t.Name;
				let value_elem = row.getElementsByClassName("tag_value")[0];
				value_elem.value = t.Value;
			} else {

				let row = document.createElement("tr");
				row.setAttribute("tag_name", t.Name);
				table.appendChild(row);
				let name_col = document.createElement("td");
				name_col.classList.add("tag_name");
				name_col.innerText = t.Name;
				row.appendChild(name_col);
				let value_col = document.createElement("td");
				row.appendChild(value_col);
				let value_input = document.createElement("input");
				value_input.classList.add("tag_value");
				value_col.appendChild(value_input);
				value_input.value = t.Value;
				let socket = this.socket;
				let get_cookie = this.get_cookie;
				value_input.addEventListener("change", function () {
					let msg = {
						Message: "WriteTag",
						Params: {
							Tags: [
								{
									Name: t.Name,
									Value: value_input.value
								}
							]
						},
						ClientCookie: get_cookie()
					};
					socket.send(JSON.stringify(msg));

				});
			}
		}
	}

	subscribe(tag_str) {
		tags = tag_str.split(" ,");
		this.socket.send(JSON.stringify({
			Message: "SubscribeTag",
			Params: {
				Tags: tags
			},
			ClientCookie: this.get_cookie()
		}));
	}

}

